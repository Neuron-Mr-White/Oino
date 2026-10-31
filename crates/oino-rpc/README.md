<img src="https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-128.png" alt="oino" width="64">

# oino-rpc

The JSONL RPC protocol for Oino sessions: codec, server over any `SessionBackend`, typed
client.

* `jsonl` — newline-delimited JSON over any reader/writer
* `wire` — client events on the wire, filtered per connection capabilities
* `server` — one line in, reply plus events out, over any backend
* `client` — typed async client with subscriptions
