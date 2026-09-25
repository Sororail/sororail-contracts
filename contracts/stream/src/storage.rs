use soroban_sdk::{contracttype, Env};
use sororail_common::{storage as ttl, Error};

use crate::types::Stream;

#[contracttype]
pub enum DataKey {
    /// The single stream held by this instance.
    Stream,
}

/// Whether `create` has run.
pub fn is_initialized(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Stream)
}

/// Loads the stream for a mutating operation and extends the instance TTL.
pub fn load(env: &Env) -> Result<Stream, Error> {
    let stream = env
        .storage()
        .instance()
        .get(&DataKey::Stream)
        .ok_or(Error::NotInitialized)?;
    ttl::extend_instance(env);
    Ok(stream)
}

/// Loads the stream without changing ledger state.
pub fn load_view(env: &Env) -> Result<Stream, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Stream)
        .ok_or(Error::NotInitialized)
}

/// Persists the stream and extends the instance TTL.
pub fn save(env: &Env, stream: &Stream) {
    env.storage().instance().set(&DataKey::Stream, stream);
    ttl::extend_instance(env);
}
