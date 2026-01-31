#pragma once
#include <netlistmgr.h>

// ---------------------------------------------------------
// 实现 INetworkListManagerEvents 接口
// 这是 COM 回调类，当网络变化时，系统会调用这里的方法
// ---------------------------------------------------------
class CNetworkListManagerEvents : public INetworkListManagerEvents {
private:
	LONG m_lRef; // 线程安全的引用计数
public:
	CNetworkListManagerEvents();
	~CNetworkListManagerEvents();
	ULONG STDMETHODCALLTYPE AddRef();
	ULONG STDMETHODCALLTYPE Release();
	HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** ppvObject);
	HRESULT STDMETHODCALLTYPE ConnectivityChanged(NLM_CONNECTIVITY newConnectivity);
	HRESULT STDMETHODCALLTYPE IsConnectedToInternetChanged(VARIANT_BOOL isConnected);
	HRESULT STDMETHODCALLTYPE IsConnectivityLowChanged(VARIANT_BOOL isLow);
	HRESULT STDMETHODCALLTYPE IsDefaultConnectivityChanged(VARIANT_BOOL isDefault);
};