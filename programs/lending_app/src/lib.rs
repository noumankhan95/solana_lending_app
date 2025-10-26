use anchor_lang::prelude::*;
pub mod instructions;
pub mod states;
use instructions::*;
use states::*;
declare_id!("AxHR9RZLDqCWg1CvPRbS5pTd73cGg6G7NzpwszhpTEKE");

#[program]
pub mod lending_app {
    use super::*;
    pub fn init_bank(
        ctx: Context<InitBank>,
        liquidation_threshold: u64,
        max_ltv: u64,
    ) -> Result<()> {
        process_init_bank(ctx, liquidation_threshold, max_ltv);
        Ok(())
    }
    pub fn init_user(ctx: Context<InitUser>, usdc_address: Pubkey) -> Result<()> {
        process_init_User(ctx, usdc_address);
        Ok(())
    }
}
