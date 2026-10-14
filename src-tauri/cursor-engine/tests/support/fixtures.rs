//! Provides isolated stores, offered models and canonical message fixtures for tests.
#![allow(dead_code)]

use std::{cell::RefCell, sync::Arc};

use cursor_engine::{
    cursor::{prompting::PromptCompiler, TransportRegistry},
    host::Host,
    model::{CanonicalMessage, ModelConfig, ModelConfigInput, Origin, Role},
    provider::Provider,
    search::WebCache,
    store::Store,
    Error,
};

thread_local! {
    /// Models a test offered before building its registry.
    static OFFERED: RefCell<Vec<ModelConfig>> = const { RefCell::new(Vec::new()) };
}

pub async fn temp_store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("test.db")).await.unwrap();
    OFFERED.with(|offered| offered.borrow_mut().clear());
    (directory, store)
}

/// Offers a model to the next registry this test builds.
pub async fn model(input: &ModelConfigInput) -> Result<ModelConfig, Error> {
    let model = ModelConfig::from_input(input)?;
    OFFERED.with(|offered| offered.borrow_mut().push(model.clone()));
    Ok(model)
}

/// A registry whose host offers the models created so far in this test.
pub fn registry(store: Store, provider: Arc<dyn Provider>, compiler: PromptCompiler) -> TransportRegistry {
    let host = Host::new();
    host.set_models(OFFERED.with(|offered| offered.borrow().clone()));
    TransportRegistry::new(store, host, provider, compiler, WebCache::default())
}

pub fn user(id: &str, text: &str) -> CanonicalMessage {
    CanonicalMessage::text(id, Role::User, Origin::User, text)
}
