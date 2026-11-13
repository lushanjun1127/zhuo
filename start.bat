@echo off
setlocal
title Zhuo
cd /d "%~dp0"

REM ============================================================
REM  Zhuo one-click launcher (single window)
REM  Runs Ollama + backend + frontend together in THIS window
REM  via concurrently. Colored prefixes:
REM    [ollama]  yellow   [backend]  cyan   [web]  green
REM  Press Ctrl+C here to stop everything.
REM ============================================================

REM open the browser once vite is likely up (10s)
start /min cmd /c "timeout /t 10 /nobreak >nul & start """" http://localhost:3000"

REM if models are not configured system-wide, use these defaults
if "%ZHUO_MODEL%"=="" set "ZHUO_MODEL=huihui_ai/qwen2.5-abliterate:0.5b"
if "%ZHUO_SUMMARY_MODEL%"=="" set "ZHUO_SUMMARY_MODEL=hf.co/bartowski/Qwen2.5-Coder-3B-Instruct-abliterated-GGUF:Q4_K_M"

npm start
pause
endlocal