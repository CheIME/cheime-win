//! CheIME TSF TIP DLL.
//!
//! This is the in-process COM DLL loaded by the Text Services Framework
//! into third-party applications. It implements:
//! - ITfTextInputProcessorEx
//! - ITfKeyEventSink
//! - ITfCompositionSink
//! - DllRegisterServer / DllUnregisterServer / DllGetClassObject / DllCanUnloadNow

pub mod exports;
