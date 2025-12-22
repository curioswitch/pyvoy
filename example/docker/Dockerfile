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
