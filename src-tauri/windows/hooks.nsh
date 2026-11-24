!macro NSIS_HOOK_POSTINSTALL
  SetDetailsView show
  DetailPrint "Installing EasySRT Studio core AI runtime (FFmpeg, CUDA/CPU, Whisper, llama.cpp, VAD)..."
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --install-runtime'
  Pop $0
  IntCmp $0 0 easysrt_postinstall_ok easysrt_postinstall_warn easysrt_postinstall_warn
  easysrt_postinstall_ok:
    DetailPrint "EasySRT Studio core AI runtime installed and verified."
    Goto easysrt_postinstall_done
  easysrt_postinstall_warn:
    DetailPrint "Warning: Core runtime installation exited with code $0. Check setup-bootstrap.log or retry inside EasySRT Studio."
    IfSilent easysrt_postinstall_done
    MessageBox MB_OK|MB_ICONEXCLAMATION "EasySRT Studio was installed, but downloading core AI components (FFmpeg/CUDA/Whisper) did not finish (code $0).$\r$\n$\r$\nCheck your internet connection and open 'Models' inside EasySRT Studio to complete setup."
  easysrt_postinstall_done:
!macroend
