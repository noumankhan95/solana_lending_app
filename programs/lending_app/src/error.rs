use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("User has Insufficient Funds")]
    InsufficientFunds,
}
