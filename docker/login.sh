#!/bin/bash
# One-time Google login for the meeting bot, inside the running container.
#
# Prerequisite: start the container with ENABLE_VNC=1 (entrypoint launches
# x11vnc on :99 / port 5900), tunnel that port to your machine, and open a VNC
# viewer so you can see and control the Chrome window this opens.
#
#   docker exec -it -u pwuser <container> /app/login.sh
#
# Log in to the DEDICATED bot Google account in the VNC window, complete any
# 2FA, then press Enter in this terminal to save the session and exit. The
# session is written into BOT_PROFILE_DIR (on the jarvis-data volume), so it
# survives restarts and is created with the server's IP + Linux cookie
# encryption — no cross-machine portability issues.
set -e

export DISPLAY=:99
export BOT_PROFILE_DIR="${BOT_PROFILE_DIR:-/data/jarvis/chrome-profile}"

mkdir -p "$BOT_PROFILE_DIR"

echo "[login] DISPLAY=$DISPLAY  BOT_PROFILE_DIR=$BOT_PROFILE_DIR"
cd /app/vexa-bot/core
exec node dist/login.js
