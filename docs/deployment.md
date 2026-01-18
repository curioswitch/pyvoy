# Deployment

pyvoy includes a CLI that spawns Envoy with the Python interpreter, allowing it to run
very similarly to any other application server. This should generally still work well
in Docker, however to avoid potential issues with subprocesses, we recommend
running Envoy directly.

pyvoy includes two commands to generate an Envoy config and startup script that can be
used within a `Dockerfile` to make this easier. This example builds an application using
`uv` and sets up Envoy for running.

```Dockerfile
FROM ghcr.io/astral-sh/uv:python3.14-trixie-slim AS builder

ENV UV_COMPILE_BYTECODE=1 UV_LINK_MODE=copy UV_PYTHON_DOWNLOADS=0

WORKDIR /app

COPY . /app
RUN uv sync --locked --no-dev

FROM python:3.14-slim-trixie

COPY --from=builder /app /app
ENV PATH="/app/.venv/bin:$PATH"

WORKDIR /pyvoy

RUN pyvoy server:app --address=0.0.0.0 --print-envoy-config > envoy-config.yaml
RUN pyvoy server:app --address=0.0.0.0 --print-envoy-entrypoint > entrypoint.sh && chmod +x entrypoint.sh

WORKDIR /app

CMD ["/pyvoy/entrypoint.sh", "--config-path", "/pyvoy/envoy-config.yaml"]
```

This example runs `--print-envoy-config` to generate an Envoy config transparently within the Docker
container, however it can be a good idea to instead run that locally to bootstrap an Envoy config
that you `ADD` to the container instead and check-in to your codebase. This allows you to fully
customize Envoy using all of its features. You can refer to the [Envoy documentation](https://www.envoyproxy.io/docs/envoy/latest/configuration/configuration)
to find all the options it supports.
