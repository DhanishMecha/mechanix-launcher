//! The D-Bus side: the names it uses, the exported agent, its
//! registration with bluetoothd, and reading the remote device.

pub(crate) mod agent;
pub(crate) mod device;
pub(crate) mod interface;
pub(crate) mod register;
