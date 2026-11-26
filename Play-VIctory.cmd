@echo off
setlocal EnableExtensions DisableDelayedExpansion
if not "%~1"=="" if /i not "%~1"=="pareto-residency" (
  echo Usage: Play-VIctory.cmd pareto-residency 1>&2
  exit /b 2
)
if not defined VICTORY_GAME_DIR set "VICTORY_GAME_DIR=%~dp0..\game-dump"
if not exist "%VICTORY_GAME_DIR%\game_scarlett_master.exe" (
  echo Set VICTORY_GAME_DIR to your own dump folder containing game_scarlett_master.exe. 1>&2
  exit /b 2
)
set "runner=%~dp0victory-3d-rtx-pareto-residency.exe"
if not exist "%runner%" set "runner=%~dp0target\release\victory.exe"
if not exist "%runner%" (
  echo Extract the Windows release or run cargo build --release --locked -p victory. 1>&2
  exit /b 2
)
if not defined VICTORY_AMBIENT_AIR set "VICTORY_AMBIENT_AIR=1.5"
if not defined VICTORY_CACHE_DIR set "VICTORY_CACHE_DIR=%LOCALAPPDATA%\VIctory\cache-pareto"
if not defined VICTORY_TEXTURE_NO_WAIT set "VICTORY_TEXTURE_NO_WAIT=1"
if not defined VICTORY_RELAY set "VICTORY_RELAY=2"
if not defined VICTORY_WIDE_BACK set "VICTORY_WIDE_BACK=1"
if not defined VICTORY_CARD_REHASH_RUNS set "VICTORY_CARD_REHASH_RUNS=1"
if not defined VICTORY_JOIN_COPIES set "VICTORY_JOIN_COPIES=1"
if not defined VICTORY_INDIRECT_WINDOW set "VICTORY_INDIRECT_WINDOW=1"
if not defined VICTORY_RELAY_COPY set "VICTORY_RELAY_COPY=1"
if not defined VICTORY_CARD_STEPS set "VICTORY_CARD_STEPS=2"
if not defined VICTORY_INDIRECT_BATCH set "VICTORY_INDIRECT_BATCH=2"
if not defined VICTORY_MAP_THREADS set "VICTORY_MAP_THREADS=1"
if not defined VICTORY_CARD_AWAKE set "VICTORY_CARD_AWAKE=1"
if not defined VICTORY_SHOW_AFTER set "VICTORY_SHOW_AFTER=1"
if not defined VICTORY_AMBIENT set "VICTORY_AMBIENT=0.8"
if not defined VICTORY_THIN set "VICTORY_THIN=1"
if not defined VICTORY_JOIN_PARTS set "VICTORY_JOIN_PARTS=8"
if not defined VICTORY_UPLOAD_AHEAD set "VICTORY_UPLOAD_AHEAD=1"
if not defined VICTORY_LEVELS_ON_DEMAND set "VICTORY_LEVELS_ON_DEMAND=1"
if not defined VICTORY_CARD_BUFFERS_MIN_KB set "VICTORY_CARD_BUFFERS_MIN_KB=64"
rem The RTX 4080 recipe's settings (avatarmanz's issue 143): the title's own
rem upscaler at 2/3 scale, card buffers, pipeline workers, texture staging.
if not defined VICTORY_CARD_BUFFERS_MB set "VICTORY_CARD_BUFFERS_MB=2048"
if not defined VICTORY_FETCH_CHECK set "VICTORY_FETCH_CHECK=1"
if not defined VICTORY_RENDER_SCALE set "VICTORY_RENDER_SCALE=0.6666667"
if not defined VICTORY_OWN_UPSCALER set "VICTORY_OWN_UPSCALER=1"
if not defined VICTORY_PIPELINE_WAIT_MS set "VICTORY_PIPELINE_WAIT_MS=150"
if not defined VICTORY_PIPELINE_WORKERS set "VICTORY_PIPELINE_WORKERS=6"
if not defined VICTORY_PIPELINE_LOWEST set "VICTORY_PIPELINE_LOWEST=1"
if not defined VICTORY_TEXTURE_STAGE_MB set "VICTORY_TEXTURE_STAGE_MB=512"
if not defined VICTORY_TEXTURE_STAGE_MS set "VICTORY_TEXTURE_STAGE_MS=50"
if not defined VICTORY_TEXTURE_CATCHUP set "VICTORY_TEXTURE_CATCHUP=1"
rem Measured on an RTX 5080 on 2026-11-27, each by an in-run A/B (README).
if not defined VICTORY_FILL_TITLE set "VICTORY_FILL_TITLE=2"
if not defined VICTORY_BURY_ASIDE set "VICTORY_BURY_ASIDE=1"
if not defined VICTORY_PLACED_ORDER set "VICTORY_PLACED_ORDER=1"
if not defined VICTORY_PORT_GUARD set "VICTORY_PORT_GUARD=1"
if not defined VICTORY_LAND_ASIDE set "VICTORY_LAND_ASIDE=1"
rem Shader writes to write-combining memory stay on the card; declared ranges
rem up to 1 MiB are still fetched back (README: -3.9 ms, same A/B).
if not defined VICTORY_SKIP_WC_BACK set "VICTORY_SKIP_WC_BACK=1"
if not defined VICTORY_SKIP_WC_KEEP set "VICTORY_SKIP_WC_KEEP=1048576"
if not defined VICTORY_LOG if not defined RUST_LOG set "VICTORY_LOG=warn"
"%runner%" run "%VICTORY_GAME_DIR%\game_scarlett_master.exe" --console series-s --execute
exit /b %errorlevel%
