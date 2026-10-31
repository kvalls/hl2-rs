//! A modal contribution supplies its lifetime and accept/cancel effects.
use crate::{
    console::{Console, Effect},
    gameui::localize::Localization,
};
use source_ui::message_box::MessageBox;
use std::sync::Arc;
pub trait ConfirmationHandler: std::fmt::Debug + Send + Sync {
    fn kind(&self) -> &'static str;
    fn message(&self, localization: &Localization, now: f64) -> MessageBox;
    fn expired(&self, now: f64) -> bool;
    fn cancel(&self, console: &mut Console) -> Vec<Effect>;
    fn accept(&self, _console: &mut Console) -> Vec<Effect> {
        Vec::new()
    }
}
#[derive(Clone, Debug)]
pub struct Confirmation(pub Arc<dyn ConfirmationHandler>);
impl PartialEq for Confirmation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
