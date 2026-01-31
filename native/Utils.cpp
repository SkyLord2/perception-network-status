#include "Utils.h"
#include <Windows.h>

// 将 0-100 的信号质量转换为大概的 RSSI (dBm)
// 这是一个经验公式，Windows 内部转换大概如此：RSSI = (Quality / 2) - 100
int QualityToRSSI(unsigned long quality) {
	if (quality == 0) return -100;
	if (quality >= 100) return -50;
	return (int)((double)quality / 2.0) - 100;
}

std::string WcharToUtf8(const wchar_t* wstr) {
	if (wstr == nullptr) return "";
	int size_needed = WideCharToMultiByte(CP_UTF8, 0, wstr, -1, nullptr, 0, nullptr, nullptr);
	if (size_needed == 0) return "";

	std::string result(size_needed, 0);
	WideCharToMultiByte(CP_UTF8, 0, wstr, -1, &result[0], size_needed, nullptr, nullptr);
	// 移除末尾的 null 终止符
	if (!result.empty() && result[result.size() - 1] == '\0') {
		result.pop_back();
	}
	return result;
}