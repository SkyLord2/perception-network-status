use std::ffi::c_void;
use std::ptr::null_mut;

use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, WIN32_ERROR};
use windows::Win32::NetworkManagement::WiFi::{
    L2_NOTIFICATION_DATA, WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST,
    WLAN_NOTIFICATION_SOURCE_MSM, WLAN_NOTIFICATION_SOURCE_NONE, WLAN_OPCODE_VALUE_TYPE,
    WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle, WlanQueryInterface,
    WlanRegisterNotification, wlan_intf_opcode_current_connection, wlan_notification_msm_connected,
    wlan_notification_msm_disconnected, wlan_notification_msm_signal_quality_change,
};
use windows::core::{Error as WinError, GUID, HRESULT, Result as WinResult};

use crate::global::{ARGS, SignalMonitorContext, with_monitor_state};
use crate::messages::send_wlan_status_message;
use crate::{report_error_log, report_info_log};

const DEFAULT_SIGNAL_DROP: u32 = 30;
const DEFAULT_SIGNAL_RECOVER: u32 = 40;

// 初始化 WLAN 监控：打开句柄、注册回调并推送一次当前信号
pub fn initialize_wlan_monitor() -> WinResult<()> {
    let mut negotiated_version = 0u32;
    let mut wlan_handle = HANDLE(null_mut());
    let open_result = unsafe { WlanOpenHandle(2, None, &mut negotiated_version, &mut wlan_handle) };
    check_win32(WIN32_ERROR(open_result), "WlanOpenHandle")?;

    let mut interface_list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
    let enum_result = unsafe { WlanEnumInterfaces(wlan_handle, None, &mut interface_list) };
    check_win32(WIN32_ERROR(enum_result), "WlanEnumInterfaces")?;

    let interface_guid = extract_first_interface_guid(interface_list);

    if !interface_list.is_null() {
        unsafe { WlanFreeMemory(interface_list as *mut c_void) };
    }

    let (threshold_drop, threshold_recover) = resolve_signal_thresholds();
    let mut context = Box::new(SignalMonitorContext {
        wlan_handle,
        threshold_drop,
        threshold_recover,
        is_signal_weak: false,
        last_quality: 0,
    });
    let context_ptr = context.as_mut() as *mut SignalMonitorContext as *mut c_void;

    with_monitor_state(|state| {
        state.wlan_handle = Some(wlan_handle);
        state.signal_context = Some(context);
    });

    let register_result = unsafe {
        WlanRegisterNotification(
            wlan_handle,
            WLAN_NOTIFICATION_SOURCE_MSM,
            true,
            Some(wlan_notification_callback),
            Some(context_ptr),
            None,
            None,
        )
    };
    check_win32(WIN32_ERROR(register_result), "WlanRegisterNotification")?;

    if let Some(guid) = interface_guid
        && let Some((quality, rssi)) = query_interface_signal(wlan_handle, &guid)
    {
        send_wlan_status_message(quality, rssi);
    }

    Ok(())
}

// 释放 WLAN 监控资源：注销通知并关闭句柄
pub fn cleanup_wlan_monitor() {
    with_monitor_state(|state| {
        if let Some(handle) = state.wlan_handle {
            let _ = unsafe {
                WlanRegisterNotification(
                    handle,
                    WLAN_NOTIFICATION_SOURCE_NONE,
                    true,
                    None,
                    None,
                    None,
                    None,
                )
            };
            let _ = unsafe { WlanCloseHandle(handle, None) };
        }

        state.wlan_handle = None;
        state.signal_context = None;
    });
}

// WLAN 通知回调：根据事件类型拉取信号并派发消息
unsafe extern "system" fn wlan_notification_callback(
    notification_data: *mut L2_NOTIFICATION_DATA,
    context: *mut c_void,
) {
    if notification_data.is_null() || context.is_null() {
        return;
    }

    let notification = unsafe { &*notification_data };
    if notification.NotificationSource != WLAN_NOTIFICATION_SOURCE_MSM {
        return;
    }

    let context = unsafe { &mut *(context as *mut SignalMonitorContext) };
    let interface_guid = &notification.InterfaceGuid;

    if notification.NotificationCode == wlan_notification_msm_disconnected.0 as u32 {
        context.last_quality = 0;
        context.is_signal_weak = false;
        send_wlan_status_message(0, 0);
        return;
    }

    if (notification.NotificationCode == wlan_notification_msm_connected.0 as u32
        || notification.NotificationCode == wlan_notification_msm_signal_quality_change.0 as u32)
        && let Some((quality, rssi)) = query_interface_signal(context.wlan_handle, interface_guid)
    {
        update_signal_state(context, quality);
        send_wlan_status_message(quality, rssi);
    }
}

// 从接口列表提取首个 WLAN 接口 GUID
fn extract_first_interface_guid(interface_list: *mut WLAN_INTERFACE_INFO_LIST) -> Option<GUID> {
    if interface_list.is_null() {
        return None;
    }

    let list = unsafe { &*interface_list };
    if list.dwNumberOfItems == 0 {
        return None;
    }

    let interfaces = unsafe {
        std::slice::from_raw_parts(list.InterfaceInfo.as_ptr(), list.dwNumberOfItems as usize)
    };
    interfaces.first().map(|info| info.InterfaceGuid)
}

// 查询 WLAN 信号：返回质量与 RSSI（RSSI 在该结构中不可直接获取时返回 0）
fn query_interface_signal(handle: HANDLE, interface_guid: &GUID) -> Option<(u32, i32)> {
    let mut data_size = 0u32;
    let mut data_ptr: *mut c_void = null_mut();
    let mut opcode = WLAN_OPCODE_VALUE_TYPE(0);

    let query_result = unsafe {
        WlanQueryInterface(
            handle,
            interface_guid,
            wlan_intf_opcode_current_connection,
            None,
            &mut data_size,
            &mut data_ptr,
            Some(&mut opcode),
        )
    };

    if WIN32_ERROR(query_result) != ERROR_SUCCESS || data_ptr.is_null() {
        if WIN32_ERROR(query_result) != ERROR_SUCCESS {
            report_error_log!("WlanQueryInterface 失败: {:?}", query_result);
        }
        return None;
    }

    let attributes = unsafe { &*(data_ptr as *const WLAN_CONNECTION_ATTRIBUTES) };
    let quality = attributes.wlanAssociationAttributes.wlanSignalQuality;
    let rssi = 0;

    unsafe { WlanFreeMemory(data_ptr) };

    Some((quality, rssi))
}

// 根据信号质量更新弱信号状态，避免频繁抖动
fn update_signal_state(context: &mut SignalMonitorContext, quality: u32) {
    let was_weak = context.is_signal_weak;

    if quality <= context.threshold_drop {
        context.is_signal_weak = true;
    } else if quality >= context.threshold_recover {
        context.is_signal_weak = false;
    }

    context.last_quality = quality;

    if was_weak != context.is_signal_weak {
        if context.is_signal_weak {
            report_info_log!("WiFi 信号进入弱信号区间，质量={}", quality);
        } else {
            report_info_log!("WiFi 信号恢复，质量={}", quality);
        }
    }
}

// 从初始化参数解析阈值，未提供时使用默认值
fn resolve_signal_thresholds() -> (u32, u32) {
    let args = ARGS.load(std::sync::atomic::Ordering::SeqCst);
    let drop = args & 0xFFFF;
    let recover = (args >> 16) & 0xFFFF;

    let drop = if drop == 0 { DEFAULT_SIGNAL_DROP } else { drop };
    let mut recover = if recover == 0 {
        DEFAULT_SIGNAL_RECOVER
    } else {
        recover
    };

    if recover <= drop {
        recover = drop + 5;
    }

    (drop, recover)
}

// 将 WIN32_ERROR 转换为 Result，并包含上下文信息
fn check_win32(error: WIN32_ERROR, context: &str) -> WinResult<()> {
    if error == ERROR_SUCCESS {
        Ok(())
    } else {
        report_error_log!("{} 失败: {:?}", context, error);
        Err(WinError::from(HRESULT::from_win32(error.0)))
    }
}
