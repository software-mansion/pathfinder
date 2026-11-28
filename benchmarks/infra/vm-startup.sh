#!/bin/bash
# Startup script of a benchmark VM. The benchmark workflow
# (.github/workflows/benchmark.yml) creates the VM.
#
# Reads its job from instance metadata:
#   bench-run-prefix  gs:// prefix of this run; the harness binary is at
#                     <prefix>/bench
#   bench-binaries    space-separated LABEL=gs://... pathfinder binaries
#   bench-iterations  value of `bench run --iterations`
#
# Writes the results to <prefix>/results/ and its log to <prefix>/vm.log,
# every minute while it runs and once more at the end. The exit code goes to
# <prefix>/exit-code last, and then the VM powers off. The workflow waits for
# the exit code and deletes the VM.

set -euo pipefail

metadata() {
    curl -sSf --retry 5 -H "Metadata-Flavor: Google" \
        "http://metadata.google.internal/computeMetadata/v1/instance/attributes/$1"
}

work_dir=/opt/bench
log=$work_dir/vm.log
mkdir -p "$work_dir/bin"

upload_log() {
    gcloud storage cp "$log" "$run_prefix/vm.log" >/dev/null 2>&1
}

# Uploads the log every minute until it gets TERM. An upload in progress
# finishes first, so it can't overwrite the final log. `wait` returns on the
# signal at once, a plain `sleep` would hold it for up to a minute.
upload_log_every_minute() {
    trap 'exit 0' TERM
    while true; do
        sleep 60 &
        wait $!
        upload_log || true
    done
}

# Runs on every exit, also after an error. The exit code tells the workflow
# that the VM is done. Powering off tells it too, in case the uploads failed.
on_exit() {
    local exit_code=$?
    if [[ -n ${log_uploader:-} ]]; then
        kill "$log_uploader" 2>/dev/null || true
        wait "$log_uploader" 2>/dev/null || true
    fi
    # Give tee a moment to write the last lines to the log.
    sleep 1
    if [[ -n ${run_prefix:-} ]]; then
        upload_log || true
        echo "$exit_code" | gcloud storage cp - "$run_prefix/exit-code" || true
    fi
    poweroff
}
trap on_exit EXIT

# The log goes to a file for the bucket, and on to the serial console, where
# the workflow can read it if the VM never writes an exit code.
exec > >(tee -a "$log") 2>&1

run_prefix=$(metadata bench-run-prefix)
upload_log_every_minute &
log_uploader=$!

# Package updates in the background would add noise to the measurements.
systemctl stop unattended-upgrades.service apt-daily.timer apt-daily-upgrade.timer || true

gcloud storage cp "$run_prefix/bench" "$work_dir/bin/bench"
chmod +x "$work_dir/bin/bench"

run_args=()
for entry in $(metadata bench-binaries); do
    label=${entry%%=*}
    mkdir -p "$work_dir/bin/$label"
    gcloud storage cp "${entry#*=}" "$work_dir/bin/$label/pathfinder"
    chmod +x "$work_dir/bin/$label/pathfinder"
    run_args+=(--binary "$label=$work_dir/bin/$label/pathfinder")
done

"$work_dir/bin/bench" run "${run_args[@]}" \
    --iterations "$(metadata bench-iterations)" \
    --output-dir "$work_dir/runs"

# The runs directory starts empty, and `bench run` creates one directory in it.
run_dir=$(find "$work_dir/runs" -mindepth 1 -maxdepth 1 -type d)

compare_args=(--target target)
if [[ -d $work_dir/bin/baseline ]]; then
    compare_args+=(--baseline baseline)
fi
"$work_dir/bin/bench" compare "$run_dir" "${compare_args[@]}"

gcloud storage cp "$run_dir"/* "$run_prefix/results/"
