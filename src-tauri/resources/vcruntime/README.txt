MSVC C++ Runtime DLL staging directory for Windows NSIS builds.
On CI (windows-latest), Microsoft Visual C++ Redistributable DLLs
(vcruntime140.dll, vcruntime140_1.dll, msvcp140.dll, vcomp140.dll, etc.)
are copied here before `tauri build` so bundled whisper-cli.exe and
llama-server.exe run on clean Windows 11 systems without external installers.
