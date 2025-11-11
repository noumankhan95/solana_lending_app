use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("User has Insufficient Funds")]
    InsufficientFunds,
    #[msg("Invalid Account")]
    InvalidPythAccount,
    #[msg("Stale Price")]
    StalePrice,
    #[msg("Over Borrowing")]
    OverBorrow,
    #[msg("Over Repaying")]
    OverRepay,
    #[msg("User has Healthy Collateral")]
    NotUndercollateralized,
}
