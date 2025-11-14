FROM ghcr.io/astral-sh/uv:python3.14-trixie-slim AS builder

ENV UV_COMPILE_BYTECODE=1 UV_LINK_MODE=copy UV_PYTHON_DOWNLOADS=0

WORKDIR /app

COPY . /app
RUN uv sync --locked --no-dev

FROM python:3.14-slim-trixie

COPY --from=builder /app /app
ENV PATH="/app/.venv/bin:$PATH"

WORKDIR /app

RUN mkdir /pyvoy && pyvoy server:app --address=0.0.0.0 --print-envoy-config > /pyvoy/envoy-config.yaml

ENV ENVOY_DYNAMIC_MODULES_SEARCH_PATH=/app/.venv/lib/python3.14/site-packages/pyvoy/_bin LD_PRELOAD=/usr/local/lib/libpython3.14.so
CMD ["/app/.venv/lib/python3.14/site-packages/pyvoy/_bin/envoy", "--config-path", "/pyvoy/envoy-config.yaml"]