//! Seerr (merged Overseerr/Jellyseerr) media-request integration.
//! Backend only: REST client + plain-data models. UI wiring lives elsewhere.

pub mod client;
pub mod models;

#[cfg(test)]
pub(crate) mod testutil;
