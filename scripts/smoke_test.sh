#!/usr/bin/env bash

# Starts Pathfinder and verifies its health, version, and sync progress.

set -euo pipefail

if [ "$#" -ne 3 ] || { [ "$1" != "--binary" ] && [ "$1" != "--image" ]; }; then
    echo "Usage: $0 <--binary|--image> <path-or-image> <expected-pathfinder-version>" >&2
    exit 2
fi

MODE=$1
SUBJECT=$2
EXPECTED_PATHFINDER_VERSION=$3
DEFAULT_ETHEREUM_API_URL=wss://ethereum-sepolia-rpc.publicnode.com
ETHEREUM_API_URL=${ETHEREUM_API_URL:-$DEFAULT_ETHEREUM_API_URL}
TARGET_BLOCK=${TARGET_BLOCK:-100}
STARTUP_TIMEOUT_SECONDS=${STARTUP_TIMEOUT_SECONDS:-120}
SYNC_TIMEOUT_SECONDS=${SYNC_TIMEOUT_SECONDS:-600}
RPC_PORT=${RPC_PORT:-19545}
MONITOR_PORT=${MONITOR_PORT:-19546}
CONTAINER_NAME="pathfinder-smoke-${RANDOM}-${RANDOM}"
WORK_DIR=''
PATHFINDER_PID=''

for command in curl jq; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Error: required command '$command' was not found" >&2
        exit 2
    fi
done

if [ "$MODE" = "--image" ] && ! command -v docker >/dev/null 2>&1; then
    echo "Error: required command 'docker' was not found" >&2
    exit 2
fi

for value in TARGET_BLOCK STARTUP_TIMEOUT_SECONDS SYNC_TIMEOUT_SECONDS RPC_PORT MONITOR_PORT; do
    if ! [[ ${!value} =~ ^[0-9]+$ ]]; then
        echo "Error: $value must be a non-negative integer" >&2
        exit 2
    fi
done

cleanup() {
    status=$?
    trap - EXIT
    set +e

    if [ "$MODE" = "--image" ]; then
        if docker inspect "$CONTAINER_NAME" >/dev/null 2>&1; then
            if [ "$status" -ne 0 ]; then
                echo "Pathfinder container logs:"
                docker logs "$CONTAINER_NAME"
            fi
            docker rm --force --volumes "$CONTAINER_NAME" >/dev/null
        fi
    elif [ -n "$WORK_DIR" ]; then
        if [ "$status" -ne 0 ] && [ -f "$WORK_DIR/pathfinder.log" ]; then
            echo "Pathfinder logs:"
            cat "$WORK_DIR/pathfinder.log"
        fi
        if [ -n "$PATHFINDER_PID" ]; then
            kill "$PATHFINDER_PID" >/dev/null 2>&1
            wait "$PATHFINDER_PID" >/dev/null 2>&1
        fi
        rm -rf "$WORK_DIR"
    fi

    exit "$status"
}
trap cleanup EXIT

if [ "$MODE" = "--image" ]; then
    docker run --detach --name "$CONTAINER_NAME" \
        --publish "127.0.0.1:${RPC_PORT}:9545" \
        --publish "127.0.0.1:${MONITOR_PORT}:9546" \
        --env "PATHFINDER_ETHEREUM_API_URL=${ETHEREUM_API_URL}" \
        --env PATHFINDER_NETWORK=sepolia-testnet \
        --env PATHFINDER_MONITOR_ADDRESS='[::]:9546' \
        --env PATHFINDER_RPC_COMPILER_CONCURRENCY_LIMIT=1 \
        "$SUBJECT" >/dev/null
else
    if [ ! -x "$SUBJECT" ]; then
        echo "Error: '$SUBJECT' is not an executable file" >&2
        exit 2
    fi

    WORK_DIR=$(mktemp -d)
    mkdir -p "$WORK_DIR/data"
    PATHFINDER_DATA_DIRECTORY="$WORK_DIR/data" \
    PATHFINDER_ETHEREUM_API_URL="$ETHEREUM_API_URL" \
    PATHFINDER_NETWORK=sepolia-testnet \
    PATHFINDER_HTTP_RPC_ADDRESS="127.0.0.1:${RPC_PORT}" \
    PATHFINDER_MONITOR_ADDRESS="127.0.0.1:${MONITOR_PORT}" \
    PATHFINDER_RPC_COMPILER_CONCURRENCY_LIMIT=1 \
        "$SUBJECT" >"$WORK_DIR/pathfinder.log" 2>&1 &
    PATHFINDER_PID=$!
fi

is_running() {
    if [ "$MODE" = "--image" ]; then
        [ "$(docker inspect --format '{{.State.Running}}' "$CONTAINER_NAME")" = "true" ]
    else
        kill -0 "$PATHFINDER_PID" >/dev/null 2>&1
    fi
}

deadline=$((SECONDS + STARTUP_TIMEOUT_SECONDS))
until curl --fail --silent "http://127.0.0.1:${MONITOR_PORT}/health" >/dev/null; do
    if ! is_running; then
        echo "Error: Pathfinder exited before becoming healthy" >&2
        exit 1
    fi
    if ((SECONDS >= deadline)); then
        echo "Error: Pathfinder did not become healthy within ${STARTUP_TIMEOUT_SECONDS}s" >&2
        exit 1
    fi
    sleep 2
done

rpc() {
    curl --fail --silent --show-error \
        --header 'Content-Type: application/json' \
        --data "$1" \
        "http://127.0.0.1:${RPC_PORT}/rpc/v0_10"
}

pathfinder_rpc() {
    curl --fail --silent --show-error \
        --header 'Content-Type: application/json' \
        --data "$1" \
        "http://127.0.0.1:${RPC_PORT}/rpc/pathfinder/v0.1"
}

# Monitoring starts before RPC, so wait for the RPC endpoint separately.
deadline=$((SECONDS + STARTUP_TIMEOUT_SECONDS))
version=''
until version="$(pathfinder_rpc '{"jsonrpc":"2.0","method":"pathfinder_version","params":[],"id":1}')"; do
    if ((SECONDS >= deadline)); then
        echo "Error: Pathfinder RPC did not become available within ${STARTUP_TIMEOUT_SECONDS}s" >&2
        exit 1
    fi
    sleep 2
done

if ! jq --exit-status --arg expected "$EXPECTED_PATHFINDER_VERSION" \
    '.error == null and .result == $expected' <<<"$version" >/dev/null; then
    echo "Error: expected Pathfinder version '$EXPECTED_PATHFINDER_VERSION', got RPC response: $version" >&2
    exit 1
fi

echo "Pathfinder is healthy and reports the expected version."

echo "Waiting up to ${SYNC_TIMEOUT_SECONDS}s to reach block ${TARGET_BLOCK}..."

deadline=$((SECONDS + SYNC_TIMEOUT_SECONDS))
last_block=-1

while ((SECONDS < deadline)); do
    response="$(rpc '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":1}')"
    if current_block="$(jq --exit-status --raw-output '.result | numbers' <<<"$response")"; then
        if [ "$current_block" -ne "$last_block" ]; then
            echo "Current block: $current_block"
            last_block=$current_block
        fi
        if [ "$current_block" -ge "$TARGET_BLOCK" ]; then
            echo "Smoke test passed at block $current_block."
            exit 0
        fi
    fi
    sleep 5
done

echo "Error: Pathfinder did not reach block ${TARGET_BLOCK} within ${SYNC_TIMEOUT_SECONDS}s (last block: ${last_block})" >&2
exit 1
