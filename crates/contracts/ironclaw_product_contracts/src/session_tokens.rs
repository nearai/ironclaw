//! Self-serve session bearer mint.
//!
//! DEMO SCOPE: self-serve bearer mint. Superseded by device-code pairing;
//! delete with the Settings Devices tab.

use serde::{Deserialize, Serialize};

use crate::descriptors::ProductSurfaceCommandDescriptor;

pub const SESSION_TOKEN_MINT_COMMAND_ID: &str = "session.tokens.mint";
pub const SESSION_TOKEN_MINT_COMMAND: ProductSurfaceCommandDescriptor<
    ProductMintSessionTokenRequest,
    ProductMintSessionTokenResponse,
> = ProductSurfaceCommandDescriptor::new(SESSION_TOKEN_MINT_COMMAND_ID);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductMintSessionTokenRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductMintSessionTokenResponse {
    pub token: String,
}
