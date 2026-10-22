#!/usr/bin/env bash
# Smoke-tests the built binary (`cargo build --locked`) from an isolated directory.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
port=${PORT:-3100}
binary_dir=$(mktemp -d)
cp "$root/target/debug/home-link" "$binary_dir/"
cd "$binary_dir"
PORT=$port DATABASE_PATH=smoke.db ./home-link &
server_pid=$!
trap 'kill -TERM "$server_pid" 2>/dev/null || true; rm -rf "$binary_dir"' EXIT

base=http://127.0.0.1:$port
for attempt in {1..20}; do
  if curl --fail --silent --show-error "$base/healthz" >/dev/null; then
    break
  fi
  if [[ "$attempt" == 20 ]]; then
    exit 1
  fi
  sleep 0.25
done

test "$(curl --fail --silent "$base/healthz")" = "ok"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml")
test "$(curl --fail --silent "$base/api/health")" = "{\"status\":\"ok\",\"version\":\"$version\"}"
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "$base/api/status")" = "503"
test "$(curl --fail --silent -X POST -H 'content-type: application/json' -d '{"name":"smoke"}' "$base/api/labels")" = '{"id":1,"name":"smoke","auto":false,"override_level":null,"override_kelvin":null,"on":null,"schedule":[],"device_count":0,"boosted":false}'
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "$base/api/missing")" = "404"
curl --fail --silent "$base/" > index.html
grep --quiet --ignore-case '<!doctype html' index.html
asset=$(sed -n 's/.*src="\([^" ]*\.js\)".*/\1/p' index.html)
test -n "$asset"
curl --fail --silent "$base$asset" > app.js
test -s app.js
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "$base/labels/1")" = "200"

kill -TERM "$server_pid"
wait "$server_pid"
echo "smoke: ok"
