use std::f32::consts::E;

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::error::ErrorCode;
use crate::states::{Bank, User};
#[derive(Accounts)]
pub struct Repay<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    pub mint: InterfaceAccount<'info, Mint>,

    #[account(mut,seeds=[mint.key().as_ref()],bump)]
    pub bank: Account<'info, Bank>,

    #[account(mut,seeds=[b"treasury",mint.key().as_ref()],bump)]
    pub bank_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut,seeds=[signer.key().as_ref()],bump)]
    pub user_account: Account<'info, User>,
    #[account(mut,associated_token::mint=mint,associated_token::authority=signer,associated_token::token_program=token_program)]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub associated_token: Program<'info, AssociatedToken>,
}

pub fn process_repay(ctx: Context<Repay>, amount: u64) -> Result<()> {
    let user = &mut ctx.accounts.user_account;
    let borrowed_val: u64;

    match ctx.accounts.mint.to_account_info().key() {
        key if key == user.usdc_address.key() => {
            borrowed_val = user.borrowed_usdc;
        }
        _ => {
            borrowed_val = user.borrowed_sol;
        }
    }
    let time_diff = user.last_updated_borrow - Clock::get()?.unix_timestamp;
    let bank = &mut ctx.accounts.bank;
    bank.total_borrow -= (bank.total_borrow as f64
        * E.powf(bank.interest_rate as f32 * time_diff as f32) as f64)
        as u64;
    let val_per_share = bank.total_borrow as f64 / bank.total_borrow_shares as f64;
    let user_value = borrowed_val / val_per_share as u64;

    if amount > user_value {
        return Err(ErrorCode::OverRepay.into());
    }

    let transfer_cpi_accounts = TransferChecked {
        from: ctx.accounts.user_token_account.to_account_info(),
        to: ctx.accounts.bank_token_account.to_account_info(),
        authority: ctx.accounts.signer.to_account_info(),
        mint: ctx.accounts.mint.to_account_info(),
    };

    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_ctx = CpiContext::new(cpi_program, transfer_cpi_accounts);
    let decimals = ctx.accounts.mint.decimals;
    transfer_checked(cpi_ctx, amount, decimals)?;
    let borrow_ratio = amount.checked_div(bank.total_borrow).unwrap();

    let user_shares = bank.total_borrow_shares.checked_mul(borrow_ratio).unwrap();

    match ctx.accounts.mint.to_account_info().key() {
        key if key == user.usdc_address => {
            user.borrowed_usdc -= amount;
            user.borrowed_usdc_shares -= user_shares;
        }
        _ => {
            user.borrowed_sol -= amount;
            user.borrowed_sol_shares -= user_shares;
        }
    }
    bank.total_borrow -= amount;
    bank.total_borrow_shares -= user_shares;
    Ok(())
}
