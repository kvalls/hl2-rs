#!/usr/bin/env bash
# Every local check in one command, no arguments: the release package build, then workspace
# validation and the Windows cross-target check (at lower CPU priority) beside the packaged
# replay suite. Prints each step's result and time and exits non-zero if any step failed.
# Environment: JOBS (replay games at once, default 3, the fastest measured on 16 GB), plus the replay suite's own settings.
# Logs: artifacts/check-all/. On Windows run the steps one by one (docs/validation.md).
set -u
cd "$(dirname "$0")/../.."
out=artifacts/check-all
mkdir -p "$out"
rm -f "$out"/*.status "$out"/*.seconds
started=$(date +%s)

# step NAME COMMAND...: runs the command into NAME.log, recording its exit status and seconds.
step() {
  local name=$1 begin
  shift
  begin=$(date +%s)
  "$@" > "$out/$name.log" 2>&1
  echo $? > "$out/$name.status"
  echo $(( $(date +%s) - begin )) > "$out/$name.seconds"
}

step build scripts/build-bevy.sh
if [ "$(cat "$out/build.status")" != 0 ]; then
  echo "build failed ($out/build.log)"
  exit 1
fi
(
  step validate nice -n 10 scripts/checks/validate.sh
  step windows nice -n 10 scripts/checks/check_windows.sh
) &
step replays env NO_BUILD=1 JOBS="${JOBS:-3}" python3 -u scripts/checks/replays.py
wait

failed=0
for name in build validate windows replays; do
  status=$(cat "$out/$name.status")
  [ "$status" = 0 ] && result=ok || { result=FAIL; failed=1; }
  printf '  %-4s  %-9s %5s s  %s\n' "$result" "$name" "$(cat "$out/$name.seconds")" "$out/$name.log"
done
tail -1 "$out/replays.log"
echo "=== all checks: $([ $failed = 0 ] && echo passed || echo FAILED) in $(( $(date +%s) - started )) s ==="
exit $failed
