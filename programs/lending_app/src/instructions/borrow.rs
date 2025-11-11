use std::f32::consts::E;

use crate::constants::{MAX_AGE, SOL_USD_PRICE_FEED, USDC_USD_PRICE_FEED};
use crate::error::ErrorCode;
use crate::states::{Bank, User};
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use pyth_sdk_solana::state::SolanaPriceAccount;
#[derive(Accounts)]

pub struct Borrow<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut,seeds=[mint.key().as_ref()],bump)]
    pub bank: Account<'info, Bank>,
    #[account(mut,seeds=[b"treasury",mint.key().as_ref()],bump)]
    pub bank_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mut,seeds=[signer.key().as_ref()],bump)]
    pub user_account: Account<'info, User>,
    #[account(init_if_needed,payer=signer,associated_token::mint=mint,associated_token::authority=signer,associated_token::token_program=token_program)]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub price_update_account: AccountInfo<'info>,
}

pub fn process_borrow(ctx: Context<Borrow>, amount: u64) -> Result<()> {
    let price_info = &ctx.accounts.price_update_account;
    let user = &mut ctx.accounts.user_account;
    let bank = &mut ctx.accounts.bank;
    let price_update: u64;
    let total_collateral: u64;

    let price_feed = SolanaPriceAccount::account_info_to_feed(price_info)
        .map_err(|_| error!(ErrorCode::InvalidPythAccount))?;
    let clock = Clock::get()?; // For current time
    let price_data = price_feed
        .get_price_no_older_than(clock.unix_timestamp, MAX_AGE)
        .ok_or(error!(ErrorCode::StalePrice))?;

    let current_price = price_data.price;
    let expo = price_data.expo;
    msg!("Price fetched: {} expo {}", current_price, expo);
    match ctx.accounts.mint.to_account_info().key() {
        key if key == user.usdc_address.key() => {
            let new_val =
                calculate_interest(user.deposited_sol, bank.interest_rate, user.last_updated)?;
            total_collateral = price_data.price as u64 * new_val;
        }
        _ => {
            let new_val =
                calculate_interest(user.deposited_usdc, bank.interest_rate, user.last_updated)?;
            total_collateral = price_data.price as u64 * new_val;
        }
    }
    let borrowable_amount = total_collateral
        .checked_mul(bank.liquidation_threshold)
        .unwrap();
    if borrowable_amount < amount {
        return Err(ErrorCode::OverBorrow.into());
    }
    let transfer_cpi_accounts = TransferChecked {
        from: ctx.accounts.bank_token_account.to_account_info(),
        authority: ctx.accounts.bank_token_account.to_account_info(),
        to: ctx.accounts.user_token_account.to_account_info(),
        mint: ctx.accounts.mint.to_account_info(),
    };
    let cpi_program = ctx.accounts.token_program.to_account_info();
    let mint_key = ctx.accounts.mint.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        b"treasury",
        mint_key.as_ref(),
        &[ctx.bumps.bank_token_account],
    ]];
    let cpi_ctx = CpiContext::new(cpi_program, transfer_cpi_accounts).with_signer(signer_seeds);
    let decimals = ctx.accounts.mint.decimals;
    transfer_checked(cpi_ctx, amount, decimals)?;
    if bank.total_borrow == 0 {
        bank.total_borrow = amount;
        bank.total_borrow_shares = amount;
    }

    let borrow_ratio = amount.checked_div(bank.total_borrow).unwrap();
    let user_shares = bank.total_borrow_shares.checked_mul(borrow_ratio).unwrap();

    match ctx.accounts.mint.to_account_info().key() {
        key if key == user.usdc_address => {
            user.borrowed_usdc += amount;
            user.borrowed_usdc_shares += user_shares;
        }
        _ => {
            user.borrowed_sol += amount;
            user.borrowed_sol_shares += user_shares;
        }
    }
    user.last_updated_borrow = Clock::get()?.unix_timestamp;
    Ok(())
}

pub fn calculate_interest(deposited: u64, interest_rate: u64, last_updated: i64) -> Result<u64> {
    let current_time = Clock::get()?.unix_timestamp;
    let time_diff = current_time - last_updated;
    let new_val =
        (deposited as f64 * E.powf(interest_rate as f32 * time_diff as f32) as f64) as u64;
    Ok(new_val)
}
