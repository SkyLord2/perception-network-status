use windows::Win32::Networking::NetworkListManager::{
    INetworkListManager, INetworkListManagerEvents, INetworkListManagerEvents_Impl,
    NLM_CONNECTIVITY, NLM_CONNECTIVITY_IPV4_INTERNET, NLM_CONNECTIVITY_IPV6_INTERNET,
    NetworkListManager,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, IConnectionPointContainer};
use windows::core::{Interface, Result as WinResult, implement};

use crate::global::{NETWORK_CONNECTED, NetworkStatus, report_network_status, with_monitor_state};
use crate::{report_error_log, report_info_log};
use std::sync::atomic::Ordering;

// NetworkListManager 事件接收器：将系统连通性变化转发到消息队列
#[implement(INetworkListManagerEvents)]
struct NetworkListManagerEvents;

impl INetworkListManagerEvents_Impl for NetworkListManagerEvents_Impl {
    fn ConnectivityChanged(&self, new_connectivity: NLM_CONNECTIVITY) -> WinResult<()> {
        log_connectivity(new_connectivity);
        let status = connectivity_to_status(new_connectivity);
        let is_connected = status != 0;

        let was_connected = NETWORK_CONNECTED.swap(is_connected, Ordering::SeqCst);
        report_info_log!(
            "当前网络状态：{}, 之前状态：{}",
            is_connected,
            was_connected
        );
        if was_connected != is_connected {
            report_network_status(NetworkStatus { status });
        }
        Ok(())
    }
}

// 初始化网络连通性监控：注册 COM 事件并推送一次当前状态
pub fn initialize_network_monitor() -> WinResult<()> {
    let network_list_manager: INetworkListManager =
        unsafe { CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)? };
    let connection_point_container: IConnectionPointContainer = network_list_manager.cast()?;
    let connection_point =
        unsafe { connection_point_container.FindConnectionPoint(&INetworkListManagerEvents::IID)? };

    let event_sink: INetworkListManagerEvents = NetworkListManagerEvents.into();
    let cookie = unsafe { connection_point.Advise(&event_sink)? };

    let connectivity = unsafe { network_list_manager.GetConnectivity() };
    let status = connectivity.map(connectivity_to_status).unwrap_or(0);
    report_info_log!("初始化网络监控，当前状态：{}", status);
    NETWORK_CONNECTED.store(status != 0, Ordering::SeqCst);

    with_monitor_state(|state| {
        state.network_list_manager = Some(network_list_manager);
        state.connection_point_container = Some(connection_point_container);
        state.connection_point = Some(connection_point);
        state.event_sink = Some(event_sink);
        state.cookie = cookie;
    });

    if status == 0 {
        report_network_status(NetworkStatus { status });
    }

    Ok(())
}

// 清理网络监控：注销事件并释放 COM 资源
pub fn cleanup_network_monitor() {
    NETWORK_CONNECTED.store(false, Ordering::SeqCst);
    with_monitor_state(|state| {
        if let Some(connection_point) = &state.connection_point
            && state.cookie != 0
            && let Err(error) = unsafe { connection_point.Unadvise(state.cookie) }
        {
            report_error_log!("注销网络事件失败: {}", error);
        }

        state.network_list_manager = None;
        state.connection_point_container = None;
        state.connection_point = None;
        state.event_sink = None;
        state.cookie = 0;
    });
}

// 将 Windows 连通性标志映射为业务状态 0/1
fn connectivity_to_status(connectivity: NLM_CONNECTIVITY) -> u32 {
    let has_internet = (connectivity.0 & NLM_CONNECTIVITY_IPV4_INTERNET.0) != 0
        || (connectivity.0 & NLM_CONNECTIVITY_IPV6_INTERNET.0) != 0;
    if has_internet { 1 } else { 0 }
}

// 输出连通性变化的详细日志，便于排查状态切换
fn log_connectivity(connectivity: NLM_CONNECTIVITY) {
    let status = connectivity_to_status(connectivity);
    report_info_log!(
        "网络连通性变化：标志={:?}，是否可用={}",
        connectivity,
        status
    );
}
