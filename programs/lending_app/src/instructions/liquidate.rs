use crate::constants::{MAX_AGE, SOL_USD_PRICE_FEED, USDC_USD_PRICE_FEED};
use crate::error::ErrorCode;
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};
use pyth_sdk_solana::state::SolanaPriceAccount;

use crate::states::{Bank, User};

#[derive(Accounts)]
pub struct Liquidate<'info> {
    #[account(mut)]
    pub liquidator: Signer<'info>,

    pub sol_price_account: AccountInfo<'info>,
    pub usdc_price_account: AccountInfo<'info>,

    pub collateral_mint: InterfaceAccount<'info, Mint>,

    pub borrowed_mint: InterfaceAccount<'info, Mint>,

    #[account(mut,seeds=[collateral_mint.key().as_ref()],bump)]
    pub collateral_bank: Account<'info, Bank>,

    #[account(mut,seeds=[borrowed_mint.key().as_ref()],bump)]
    pub borrowed_bank: Account<'info, Bank>,

    #[account(mut,seeds=[b"treasury",borrowed_mint.key().as_ref()],bump)]
    pub borrowed_bank_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mut,seeds=[b"treasury",collateral_mint.key().as_ref()],bump)]
    pub collateral_bank_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mut,seeds=[liquidator.key().as_ref()],bump)]
    pub user_account: Account<'info, User>,

    #[account(init_if_needed,payer=liquidator,associated_token::mint=collateral_mint,associated_token::authority=liquidator,associated_token::token_program=token_program)]
    pub liquidator_collateral_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(init_if_needed,payer=liquidator,associated_token::mint=borrowed_mint,associated_token::authority = liquidator,
    associated_token::token_program=token_program)]
    pub liquidator_borrowed_token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,

    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_liquidate(ctx: Context<Liquidate>) -> Result<()> {
    let collateral_bank = &mut ctx.accounts.collateral_bank;
    let borrowed_bank = &mut ctx.accounts.borrowed_bank;
    let user = &mut ctx.accounts.user_account;

    // ✅ Get current blockchain time
    let clock = Clock::get()?;

    // ✅ Fetch SOL price from Pyth feed
    let sol_price_feed = SolanaPriceAccount::account_info_to_feed(&ctx.accounts.sol_price_account)
        .map_err(|_| error!(ErrorCode::InvalidPythAccount))?;
    let sol_price_data = sol_price_feed
        .get_price_no_older_than(clock.unix_timestamp, MAX_AGE)
        .ok_or(error!(ErrorCode::StalePrice))?;
    let sol_price = sol_price_data.price;
    let sol_expo = sol_price_data.expo;

    // ✅ Fetch USDC price from Pyth feed
    let usdc_price_feed =
        SolanaPriceAccount::account_info_to_feed(&ctx.accounts.usdc_price_account)
            .map_err(|_| error!(ErrorCode::InvalidPythAccount))?;
    let usdc_price_data = usdc_price_feed
        .get_price_no_older_than(clock.unix_timestamp, MAX_AGE)
        .ok_or(error!(ErrorCode::StalePrice))?;
    let usdc_price = usdc_price_data.price;
    let usdc_expo = usdc_price_data.expo;

    msg!("SOL price: {} (expo {})", sol_price, sol_expo);
    msg!("USDC price: {} (expo {})", usdc_price, usdc_expo);

    // ✅ Calculate collateral and borrowed value
    let total_collateral: u64;
    let total_borrowed: u64;

    match ctx.accounts.collateral_mint.to_account_info().key() {
        key if key == user.usdc_address => {
            let new_usdc = calculate_interest(
                user.deposited_usdc,
                collateral_bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = (usdc_price as u64) * new_usdc;

            let new_sol = calculate_interest(
                user.deposited_sol,
                borrowed_bank.interest_rate,
                user.last_updated_borrow,
            )?;
            total_borrowed = (sol_price as u64) * new_sol;
        }
        _ => {
            let new_sol = calculate_interest(
                user.deposited_sol,
                collateral_bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = (sol_price as u64) * new_sol;

            let new_usdc = calculate_interest(
                user.borrowed_usdc,
                borrowed_bank.interest_rate,
                user.last_updated_borrow,
            )?;
            total_borrowed = (usdc_price as u64) * new_usdc;
        }
    }

    let total_collateral =
        (sol_price as u64 * user.deposited_sol) + (usdc_price as u64 * user.deposited_usdc);
    let total_borrowed =
        (sol_price as u64 * user.borrowed_sol) + (usdc_price as u64 * user.borrowed_usdc);

    let health_factor = (total_collateral * collateral_bank.liquidation_threshold) / total_borrowed;

    if health_factor >= 1 {
        return Err(ErrorCode::NotUndercollateralized.into());
    }

    let liquidation_amount = total_borrowed * collateral_bank.liquidation_close_factor;

    // liquidator pays back the borrowed amount back to the bank

    let transfer_to_bank = TransferChecked {
        from: ctx
            .accounts
            .liquidator_borrowed_token_account
            .to_account_info(),
        mint: ctx.accounts.borrowed_mint.to_account_info(),
        to: ctx.accounts.borrowed_bank_token_account.to_account_info(),
        authority: ctx.accounts.liquidator.to_account_info(),
    };

    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_ctx_to_bank = CpiContext::new(cpi_program.clone(), transfer_to_bank);
    let decimals = ctx.accounts.borrowed_mint.decimals;

    transfer_checked(cpi_ctx_to_bank, liquidation_amount, decimals)?;

    // Transfer liquidation value and bonus to liquidator
    let liquidation_bonus =
        (liquidation_amount * collateral_bank.liquidation_bonus) + liquidation_amount;

    let transfer_to_liquidator = TransferChecked {
        from: ctx.accounts.collateral_bank_token_account.to_account_info(),
        mint: ctx.accounts.collateral_mint.to_account_info(),
        to: ctx
            .accounts
            .liquidator_collateral_token_account
            .to_account_info(),
        authority: ctx.accounts.collateral_bank_token_account.to_account_info(),
    };

    let mint_key = ctx.accounts.collateral_mint.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        b"treasury",
        mint_key.as_ref(),
        &[ctx.bumps.collateral_bank_token_account],
    ]];
    let cpi_ctx_to_liquidator =
        CpiContext::new(cpi_program.clone(), transfer_to_liquidator).with_signer(signer_seeds);
    let collateral_decimals = ctx.accounts.collateral_mint.decimals;
    transfer_checked(
        cpi_ctx_to_liquidator,
        liquidation_bonus,
        collateral_decimals,
    )?;

    Ok(())
}

pub fn calculate_interest(deposited: u64, interest_rate: u64, last_updated: i64) -> Result<u64> {
    let current_time = Clock::get()?.unix_timestamp;
    let time_diff = current_time - last_updated;
    // Simplified exponential growth
    let new_val = (deposited as f64
        * f64::exp(interest_rate as f64 * time_diff as f64 / 31_536_000.0))
        as u64;
    Ok(new_val)
}
