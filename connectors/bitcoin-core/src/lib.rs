#![doc = "Bitcoin Core observer connector with source-local gap accounting."]

pub mod capture;
pub mod config;
pub mod error;
pub mod health;
pub mod reconcile;
pub mod rpc;
pub mod session;
pub mod zmq;
