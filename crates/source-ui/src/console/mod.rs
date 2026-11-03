//! The developer console every app shares, following the Source SDK's `CConsoleDialog` /
//! `CConsolePanel` (`vgui2/vgui_controls/consoledialog.cpp`): the commands and cvars an app
//! registers and runs ([`command`]), their completion ([`complete`]), the input line with its
//! history ([`editor`]), the dialog's layout ([`dialog`]) and the dialog itself with its output
//! buffer ([`panel`]). The app supplies the command table, the context its commands run on,
//! the names their arguments complete from, its scaling, texts and look.
pub mod command;
pub mod complete;
pub mod dialog;
mod editing;
pub mod editor;
pub mod panel;
