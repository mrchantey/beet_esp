# ESP Radio

Time to implement some esp radio examples.

We need to get all of this working in general, im going away for a bit so just see how far you can get without getting stuck.

create examples, for instance:

- `examples/wifi-client.rs`
- `examples/wifi-server.rs`
- `examples/bluetooth-client.rs`

And run them on the device, ensuring they work. use timeouts to avoid getting stuck for whatever reason. And maybe see what other cool connectivity stuff you can ·demonstrate with this no-std esp32-s3 setup

start with wifi-client. 
ive added the network ssid and password to .env, use those.

## Reference

ive added relevent repos for your reference
- `agent/reference/esp-hal` (see esp-radio)
- `agent/reference/embassy` (see embassy-net)
