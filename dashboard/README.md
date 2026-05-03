# Workspace UI

Local workspace console for the first product slice. It serves static
HTML/CSS/JS and a thin local API harness. The browser never calls LLM
APIs; `POST /trigger/selection` invokes `chartered-runtime` with the
selection trigger flags.

Run from a configured workspace:

```sh
WORKSPACE_ROOT=/path/to/workspace \
CHARTERED_DIR=/path/to/workspace/.chartered \
node dashboard/local-api.mjs
```

Optional:

```sh
CHARTERED_RUNTIME_BIN=/path/to/chartered-runtime
PORT=5177
```

When `CHARTERED_RUNTIME_BIN` is absent, the harness runs the runtime via
`cargo run --manifest-path runtime/Cargo.toml`.
