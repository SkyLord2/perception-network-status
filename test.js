const { doInitialize } = require('./index.js')


doInitialize(
    (status) => {
      if (status === 1) {
        console.log('network connected');  
      } else {
        console.log('network disconnected');
      } 
    },
    (strong, quality, rssi) => {
      console.log('wifi signal become strong', strong);
      console.log(`wifi quality: ${quality}%`);
      console.log(`wifi signal strength: ${rssi}dBm`);
    },
    40,
    50,
    (log) => {
      console.log("addon log: ", log);
        
    }
);

setInterval(() => {
  console.log('5分钟过去了');
}, 1000 * 60 * 5);
