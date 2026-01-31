#include "CNetworkListManagerEvents.h"

extern void PrintConnectivity(NLM_CONNECTIVITY connectivity);
extern void sendNetworkStatusMessage(NLM_CONNECTIVITY connectivity);

CNetworkListManagerEvents::CNetworkListManagerEvents() : m_lRef(1) {}
CNetworkListManagerEvents::~CNetworkListManagerEvents() {}

// --- IUnknown 标准实现 ---
ULONG STDMETHODCALLTYPE CNetworkListManagerEvents::AddRef() {
	return InterlockedIncrement(&m_lRef);
}

ULONG STDMETHODCALLTYPE CNetworkListManagerEvents::Release() {
	LONG lRef = InterlockedDecrement(&m_lRef);
	if (lRef == 0) {
		delete this;
		return 0;
	}
	return lRef;
}

HRESULT STDMETHODCALLTYPE CNetworkListManagerEvents::QueryInterface(REFIID riid, void** ppvObject) {
	if (riid == IID_IUnknown || riid == IID_INetworkListManagerEvents) {
		*ppvObject = static_cast<INetworkListManagerEvents*>(this);
		AddRef();
		return S_OK;
	}
	*ppvObject = NULL;
	return E_NOINTERFACE;
}

// --- INetworkListManagerEvents 核心回调 ---

// 关键方法：当整个机器的 Internet 连接性发生变化时触发
HRESULT STDMETHODCALLTYPE CNetworkListManagerEvents::ConnectivityChanged(NLM_CONNECTIVITY newConnectivity) {
	PrintConnectivity(newConnectivity);
	sendNetworkStatusMessage(newConnectivity);
	return S_OK;
}

// 本示例不需要处理的事件，返回 S_OK 即可
HRESULT STDMETHODCALLTYPE CNetworkListManagerEvents::IsConnectedToInternetChanged(VARIANT_BOOL isConnected) { return S_OK; } // 可以简单处理
HRESULT STDMETHODCALLTYPE CNetworkListManagerEvents::IsConnectivityLowChanged(VARIANT_BOOL isLow) { return S_OK; } // 低带宽模式变化
HRESULT STDMETHODCALLTYPE CNetworkListManagerEvents::IsDefaultConnectivityChanged(VARIANT_BOOL isDefault) { return S_OK; }