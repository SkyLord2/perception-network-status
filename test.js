const { doInitialize, enableNetQualityProb } = require('./index.js')

doInitialize(
  (err, { status }) => {
    if (status === 1) {
      console.log('网络已连接')
    } else {
      console.log('网络已断开')
    }
  },
  (err, { strong, quality, rssi }) => {
    console.log('wifi信号变强', strong)
    console.log(`wifi质量: ${quality}%`)
    console.log(`wifi信号强度: ${rssi}dBm`)
  },
  40,
  50,
  (err, info) => {
    console.log('网络质量: ', info)
  },
  (err, log) => {
    console.log('addon log: ', log)
  },
  false,
)

setTimeout(() => {
  enableNetQualityProb(true)
}, 1000 * 30)

setInterval(
  () => {
    console.log('5分钟过去了')
  },
  1000 * 60 * 5,
)
