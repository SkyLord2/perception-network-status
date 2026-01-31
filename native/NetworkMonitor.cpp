#include <windows.h>
#include <wlanapi.h>
#include <iostream>
#include <atomic>

#include "CNetworkListManagerEvents.h"
#include "Utils.h"

// ���ӱ�Ҫ�Ŀ�
#pragma comment(lib, "wlanapi.lib")
#pragma comment(lib, "ole32.lib")

#define WM_NETWORK_STATUS_CHANGE (WM_USER + 107)
#define WM_WIFI_SIGNAL_CHANGE (WM_USER + 108)

DWORD g_mainThreadId = 0;
bool g_networkConnected = false;

static void LogFunc(const std::wstring& info) {
	std::wcout << info << std::endl;
}

void LogError(const std::wstring& error) {
	std::wstring logInfo = L"[network error] " + error;
	LogFunc(logInfo);
}

void LogInfo(const std::wstring& info) {
	std::wstring logInfo = L"[network info] " + info;
	LogFunc(logInfo);
}

// ������������������ӡ����״̬
void PrintConnectivity(NLM_CONNECTIVITY connectivity) {
	std::cout << "net status changed:" << std::endl;

	if (connectivity == NLM_CONNECTIVITY_DISCONNECTED) {
		g_networkConnected = false;
		std::cout << "  [status]: No Network" << std::endl;
		return;
	}

	// ��� IPv4
	if (connectivity & NLM_CONNECTIVITY_IPV4_NOTRAFFIC)		std::cout << "  [IPv4]: no traffic" << std::endl;
	if (connectivity & NLM_CONNECTIVITY_IPV4_SUBNET)		std::cout << "  [IPv4]: subnet (no Internet)" << std::endl;
	if (connectivity & NLM_CONNECTIVITY_IPV4_LOCALNETWORK)	std::cout << "  [IPv4]: local network" << std::endl;
	if (connectivity & NLM_CONNECTIVITY_IPV4_INTERNET) {
		g_networkConnected = true;
		std::cout << "  [IPv4]: Internet connected (OK)" << std::endl;
	}

	// ��� IPv6
	if (connectivity & NLM_CONNECTIVITY_IPV6_NOTRAFFIC)		std::cout << "  [IPv6]: no traffic" << std::endl;
	if (connectivity & NLM_CONNECTIVITY_IPV6_SUBNET)		std::cout << "  [IPv6]: subnet (no Internet)" << std::endl;
	if (connectivity & NLM_CONNECTIVITY_IPV6_LOCALNETWORK)	std::cout << "  [IPv6]: local network" << std::endl;
	if (connectivity & NLM_CONNECTIVITY_IPV6_INTERNET) {
		g_networkConnected = true;
		std::cout << "  [IPv6]: Internet connected (OK)" << std::endl;
	}
}

void sendNetworkStatusMessage(NLM_CONNECTIVITY connectivity) {
	// 定义 Internet 掩码：只要 IPv4 或 IPv6 任意一个有 Internet 访问权限，就算连网
    bool hasInternet = (connectivity & NLM_CONNECTIVITY_IPV4_INTERNET) || 
                       (connectivity & NLM_CONNECTIVITY_IPV6_INTERNET);

    if (hasInternet) {
        // 有互联网访问 -> 发送状态 1
        PostThreadMessage(g_mainThreadId, WM_NETWORK_STATUS_CHANGE, 1, 0);
    } else {
        // 没有互联网访问 (包括 DISCONNECTED, LOCALNETWORK, NOTRAFFIC 等) -> 发送状态 0
        // 这样即使是 "local network" 也会被判定为断网
        PostThreadMessage(g_mainThreadId, WM_NETWORK_STATUS_CHANGE, 0, 0);
    }
}

INetworkListManager* g_pNetworkListManager = NULL;
IConnectionPointContainer* g_pCPContainer = NULL;
IConnectionPoint* g_pConnectPoint = NULL;
CNetworkListManagerEvents* g_pNetEvents = NULL;
DWORD g_dwCookie = 0;

int StartNetworkMonitor() {
	HRESULT hr = S_OK;
	
	// 1. ��ʼ�� COM �� (���߳�ģʽ)
	hr = CoInitializeEx(NULL, COINIT_MULTITHREADED);
	if (FAILED(hr)) {
		LogError(L"CoInitializeEx failed.");
		return 1;
	}

	LogInfo(L"Initializing Network List Manager...");

	// 2. ���� Network List Manager ʵ��
	hr = CoCreateInstance(CLSID_NetworkListManager, NULL,
		CLSCTX_ALL, IID_INetworkListManager,
		(void**)&g_pNetworkListManager);

	if (FAILED(hr)) {
        LogError(L"Can not create NetworkListManager instance. please check if you have installed NLM (Vista+).");
		CoUninitialize();
		return 1;
	}

	// 3. ��ȡ���ӵ����� (����ע���¼�)
	hr = g_pNetworkListManager->QueryInterface(IID_IConnectionPointContainer, (void**)&g_pCPContainer);
	if (SUCCEEDED(hr)) {
		// 4. �ҵ� INetworkListManagerEvents �����ӵ�
		hr = g_pCPContainer->FindConnectionPoint(IID_INetworkListManagerEvents, &g_pConnectPoint);
	}

	if (SUCCEEDED(hr)) {
		// 5. ʵ�������ǵ��¼�������
		g_pNetEvents = new CNetworkListManagerEvents();

		// 6. ע���¼� (Advise)
		// dwCookie ��ע��ƾ֤������ע��ʱ��Ҫ�õ�
		hr = g_pConnectPoint->Advise((IUnknown*)g_pNetEvents, &g_dwCookie);

		if (SUCCEEDED(hr)) {
			LogInfo(L"Network monitor started.");
			// 7. �ֶ���ȡһ�ε�ǰ״̬ (��Ϊ�ص�ֻ�ڱ仯ʱ����)
			NLM_CONNECTIVITY currentConnectivity;
			if (SUCCEEDED(g_pNetworkListManager->GetConnectivity(&currentConnectivity))) {
				LogInfo(L"Initial network status.");
				PrintConnectivity(currentConnectivity);
				if (currentConnectivity == NLM_CONNECTIVITY_DISCONNECTED) {
					PostThreadMessage(g_mainThreadId, WM_NETWORK_STATUS_CHANGE, 0, 0);
				}
			}
		}
		else {
			LogError(L"Advise failed.");
		}
	}
	else {
		LogError(L"Can not get connection point.");
	}
	return 0;
}

void StopNetworkMonitor() {
	// 8. ע���¼� (Unadvise) - ��һ���ǳ���Ҫ��
	// �����ע����COM ���������Զ���ᱻ�ͷ�
	if (g_pConnectPoint) 
	{
		g_pConnectPoint->Unadvise(g_dwCookie);
	}
	// �ͷ����ǵ��¼��������� (COM �ڲ� AddRef ��һ�Σ�����������Ҫ Release �Լ�������)
	if (g_pNetEvents)
	{
		g_pNetEvents->Release();
	}
	LogInfo(L"Network monitor stopped.");
	// 9. ��Դ�ͷ����� (�ϸ��շ���˳��)
	SafeRelease(&g_pConnectPoint);
	SafeRelease(&g_pCPContainer);
	SafeRelease(&g_pNetworkListManager);
	// 10. ����ʼ�� COM
	CoUninitialize();
	LogInfo(L"Program exited safely.");
}

void sendWlanStatusMessage(ULONG quality, int rssi) {
	PostThreadMessage(g_mainThreadId, WM_WIFI_SIGNAL_CHANGE, quality, rssi);
}

// ---------------------------------------------------------
// ���Ļص�������WLAN_NOTIFICATION_CALLBACK
// ϵͳ���ں�̨�̵߳��ô˺��������Ҫע���̰߳�ȫ
// ---------------------------------------------------------
void WINAPI WlanNotificationCallback(
	PWLAN_NOTIFICATION_DATA pNotificationData,
	PVOID pContext
) {
	SignalMonitorContext* pMonitor = (SignalMonitorContext*)pContext;
	if (pMonitor == nullptr) {
		std::cout << "pMonitor is null!" << std::endl;
		return;
	}
	// ����ֻ���� MSM (Media Specific Module) ��֪ͨ
	// ��Ϊ�ź������仯����ý���ض���
	if (pNotificationData->NotificationSource == WLAN_NOTIFICATION_SOURCE_MSM) {

		switch (pNotificationData->NotificationCode) {

			// [�ؼ�] �ź����������仯
		case wlan_notification_msm_signal_quality_change:
		{
			// ���ڴ��¼���pData ����һ�� ULONG ���͵�ֵ (0-100)
			if (pNotificationData->dwDataSize >= sizeof(ULONG)) {
				ULONG currentQuality = *(ULONG*)pNotificationData->pData;
				int currentRSSI = QualityToRSSI(currentQuality);
				
				// --- �����߼���״̬�� ---
				// ��� 1: ֮ǰ��ǿ�źţ����ڵ����� Drop ��ֵ����
				if (!pMonitor->isSignalWeak && currentQuality <= pMonitor->thresholdDrop) {
					pMonitor->isSignalWeak = true; // �л�״̬

					std::cout << "\n[Warning] The WiFi signal has become weak!" << std::endl;
                    std::cout << "  -> threshold: " << pMonitor->thresholdDrop << "%" << std::endl;
					std::cout << "  -> quality: " << currentQuality << "%" << std::endl;
					std::cout << "  -> RSSI: " << currentRSSI << " dBm" << std::endl;
					sendWlanStatusMessage(currentQuality, currentRSSI);
				}

				// ��� 2: ֮ǰ�����źţ����ڻָ����� Recover ��ֵ����
				// ע�⣺����ʹ�� thresholdRecover ������ thresholdDrop��ʵ���˷�����
				else if (pMonitor->isSignalWeak && currentQuality >= pMonitor->thresholdRecover) {
					pMonitor->isSignalWeak = false; // �л�״̬

					std::cout << "\n[Info] The WiFi signal has become strong!" << std::endl;
					std::cout << "  -> threshold: " << pMonitor->thresholdRecover << "%" << std::endl;
					std::cout << "  -> quality: " << currentQuality << "%" << std::endl;
					std::cout << "  -> RSSI: " << currentRSSI << " dBm" << std::endl;
					sendWlanStatusMessage(currentQuality, currentRSSI);
				}

				// ���¼�¼
				pMonitor->lastQuality = currentQuality;

				// ����нӿ� GUID��Ҳ���Դ�ӡ�����������ĸ�����
				// WCHAR guidString[40] = {0};
				// StringFromGUID2(pNotificationData->InterfaceGuid, guidString, 39);
				// std::wcout << L"  -> �ӿ�: " << guidString << std::endl;
			}
			break;
		}

		// ��������Ȥ�������¼�
		case wlan_notification_msm_connected:
			std::cout << "WiFi connected." << std::endl;
			break;

		case wlan_notification_msm_disconnected:
			std::cout << "WiFi disconnected." << std::endl;
			break;

		default:
			// �������� MSM ֪ͨ
			break;
		}
	}
}

HANDLE g_hClient = NULL;
SignalMonitorContext monitorCtx = { 40, 50, false, 100 };
int StartWlanMonitor() {
	DWORD dwResult = 0;
	DWORD dwMaxClient = 2;
	DWORD dwCurVersion = 0;

	LogInfo(L"Initializing Native Wifi Notification Listener...");

	// 1. �򿪾��
	dwResult = WlanOpenHandle(dwMaxClient, NULL, &dwCurVersion, &g_hClient);
	if (dwResult != ERROR_SUCCESS) {
		LogError(L"Failed to open handle.");
		return 1;
	}

	PWLAN_INTERFACE_INFO_LIST pIfList = NULL;
	dwResult = WlanEnumInterfaces(g_hClient, NULL, &pIfList);
	if (dwResult == ERROR_SUCCESS && pIfList->dwNumberOfItems > 0) {
		// ȡ��һ���ӿ�
		PWLAN_INTERFACE_INFO pIfInfo = &pIfList->InterfaceInfo[0];

		DWORD connectSize = 0;
		PWLAN_CONNECTION_ATTRIBUTES pConnectInfo = NULL;
		WLAN_OPCODE_VALUE_TYPE opCode;

		// ��ѯ��ǰ����״̬
		if (WlanQueryInterface(g_hClient, &pIfInfo->InterfaceGuid, wlan_intf_opcode_current_connection,
			NULL, &connectSize, (PVOID*)&pConnectInfo, &opCode) == ERROR_SUCCESS) {

			if (pConnectInfo->isState == wlan_interface_state_connected) {
				unsigned long startQuality = pConnectInfo->wlanAssociationAttributes.wlanSignalQuality;

				// ���ݵ�ǰʵ��ֵ��ʼ��״̬
				if (startQuality <= monitorCtx.thresholdDrop) {
					monitorCtx.isSignalWeak = true;
					std::cout << "  -> ��ʼ���: ��ǰ�ź�΢�� (" << startQuality << "%), �ѽ���[���ź�]ģʽ��" << std::endl;
				}
				else {
					monitorCtx.isSignalWeak = false;
					std::cout << "  -> ��ʼ���: ��ǰ�ź����� (" << startQuality << "%), �ѽ���[���]ģʽ��" << std::endl;
				}
				monitorCtx.lastQuality = startQuality;
			}
			WlanFreeMemory(pConnectInfo);
		}
		WlanFreeMemory(pIfList);
	}

	// 2. ע��֪ͨ
	// WLAN_NOTIFICATION_SOURCE_MSM: �����ź����������ӡ��Ͽ���������/��·��䶯
	// WLAN_NOTIFICATION_SOURCE_ACM: �����Զ����á�ɨ����ɵȱ䶯
	// �������ǽ����߶�ע�ᣬ�Ի����ȫ����Ϣ
	dwResult = WlanRegisterNotification(
		g_hClient,
		WLAN_NOTIFICATION_SOURCE_MSM,
		FALSE,                  // �����ظ�֪ͨ
		WlanNotificationCallback, // ���ǵĻص�����
		&monitorCtx,                   // ������ָ�� (���Դ� this ���Զ���ṹ��)
		NULL,                   // ����
		NULL                    // ֮ǰ��֪ͨԴ (������)
	);

	if (dwResult != ERROR_SUCCESS) {
		LogError(L"Failed to register notification.");
		WlanCloseHandle(g_hClient, NULL);
		return 1;
	}

	LogInfo(L"Native Wifi Notification Listener started.");
	//std::cout << "ע�⣺�ź�֪ͨ�Ĵ���Ƶ��ȡ��������������ͨ�������ź���������ʱ������" << std::endl;
	//std::cout << "�볢���ƶ��ʼǱ�λ�ã����ڵ������Դ����仯..." << std::endl;
	//std::cout << "�� Enter ���˳�����" << std::endl;
	return 0;
}

void StopWlanMonitor() {
	// 4. ������Դ
	LogInfo(L"Cleaning up wlan resources...");

	// ȡ��ע�� (��Ȼ CloseHandle ���Զ�����������ʽȡ���Ǻ�ϰ��)
	WlanRegisterNotification(
		g_hClient,
		WLAN_NOTIFICATION_SOURCE_NONE,
		FALSE,
		NULL,
		NULL,
		NULL,
		NULL
	);

	WlanCloseHandle(g_hClient, NULL);
}

void reportNetworkStatus(int status) {
	
}

void reportWifiSignal(int quality, int rssi) {
	
}

static void OnExit(void* arg) {
	StopNetworkMonitor();
	StopWlanMonitor();
	LogInfo(L"Monitoring stopped by process exit");
}

int main() {
	g_mainThreadId = GetCurrentThreadId();
	std::cout << "��ǰ�߳� ID: " << g_mainThreadId << std::endl;
	StartNetworkMonitor();
	StartWlanMonitor();
	// �������̣߳��ȴ��¼��ص�
	MSG msg;
	while (GetMessage(&msg, NULL, 0, 0))
	{
		if (msg.message == WM_NETWORK_STATUS_CHANGE) {
			std::cout << "network status changed: " << msg.wParam << std::endl;
		}
		else if (msg.message == WM_WIFI_SIGNAL_CHANGE) {
			std::cout << "Wifi signal changed quality: " << msg.wParam << ", RSSI: " << msg.lParam << std::endl;
		}
		TranslateMessage(&msg);
		DispatchMessage(&msg);
	}
	StopNetworkMonitor();
	StopWlanMonitor();
	return 0;
}