#pragma once
#include <string>

// ---------------------------------------------------------
// 上下文结构体：用于在回调函数中保存状态
// ---------------------------------------------------------
struct SignalMonitorContext {
	// 配置：阈值 (0-100)
	// 建议设置两个阈值以形成"迟滞区间"，防止信号在临界值反复跳动导致事件频繁触发
	unsigned long thresholdDrop;    // 低于此值，视为变弱 (Trigger: Strong -> Weak)
	unsigned long thresholdRecover; // 高于此值，视为变强 (Trigger: Weak -> Strong)
	// 状态：当前是否处于弱信号状态
	bool isSignalWeak;
	// 辅助：记录上一次的具体数值，仅用于日志对比
	unsigned long lastQuality;
};

// 辅助函数：安全释放 COM 接口指针
template <class T> 
void SafeRelease(T** ppT) {
	if (*ppT) {
		(*ppT)->Release();
		*ppT = NULL;
	}
}

int QualityToRSSI(unsigned long quality);

std::string WcharToUtf8(const wchar_t* wstr);