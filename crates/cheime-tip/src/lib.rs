//! CheIME TSF TIP DLL.
//!
//! This is the in-process COM DLL loaded by the Text Services Framework
//! into third-party applications. It implements:
//! - ITfTextInputProcessorEx (ActivateEx / Deactivate)
//! - ITfKeyEventSink (OnTestKeyDown / OnKeyDown)
//! - ITfCompositionSink (OnCompositionTerminated)
//! - ITfDisplayAttributeProvider (preedit styling)
//! - ITfThreadMgrEventSink (focus / thread changes)
//! - DllRegisterServer / DllUnregisterServer / DllGetClassObject / DllCanUnloadNow

#![windows_subsystem = "console"]
