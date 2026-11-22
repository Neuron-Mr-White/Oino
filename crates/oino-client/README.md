<img src="https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-128.png" alt="oino" width="64">

# oino-client

Client contract for Oino sessions: [`SessionBackend`], RPC command records, in-process and
fake backends.

* `protocol` — [`RpcCommand`] (agent session + `_oino/*` commands), responses, errors
* `backend` — the [`SessionBackend`] trait every client talks to
* `in-process` (feature) — an in-process backend over an `oino-core` runtime
* `frontend` — typed `_oino/*` frontend commands (completions, panels, shortcuts, settings…)
* `fake` — a scripted backend for tests
