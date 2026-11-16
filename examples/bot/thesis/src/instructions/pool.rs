use super::{ata, key, pda, Result, AMM, BUYBACK, SPL, WSOL};
use solana_sdk::{
    account::Account,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub struct Pool {
    address: Pubkey,
    base_mint: Pubkey,
    base_vault: Pubkey,
    quote_vault: Pubkey,
}

impl Pool {
    /// Parse and validate the pool account for the requested mint pair.
    pub fn parse(address: Pubkey, account: &Account, base: Pubkey, quote: Pubkey) -> Result<Self> {
        if account.owner != key(AMM)
            || account.data.get(..8) != Some(&[241, 154, 109, 4, 17, 177, 109, 188])
        {
            return Err("invalid canonical PumpSwap pool".into());
        }

        // Reject a canonical pool for any mint pair other than the requested SOL pair.
        let data = &account.data;
        let pool_base = super::account_key(data, 43)?;
        let pool_quote = super::account_key(data, 75)?;
        if pool_base != base || pool_quote != quote {
            return Err("PumpSwap pool mint pair does not match".into());
        }

        Ok(Self {
            address,
            base_mint: base,
            base_vault: super::account_key(data, 139)?,
            quote_vault: super::account_key(data, 171)?,
        })
    }
}

/// Build the PumpSwap buy or sell instruction.
pub fn instruction(
    pool: &Pool,
    user: Pubkey,
    token_program: Pubkey,
    buy: bool,
    amount: u64,
    min_out: u64,
) -> Instruction {
    let program = key(AMM);
    let quote = key(WSOL);
    let data = instruction_data(buy, amount, min_out);

    Instruction {
        program_id: program,
        accounts: accounts(pool, user, token_program, quote, program),
        data,
    }
}

/// Encode the discriminator and arguments for a pool swap.
fn instruction_data(buy: bool, amount: u64, min_out: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(if buy {
        &[194, 171, 28, 70, 104, 77, 91, 47]
    } else {
        &[93, 246, 130, 60, 231, 233, 64, 178]
    });
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&min_out.to_le_bytes());
    data
}

/// List the accounts required by the PumpSwap instruction.
fn accounts(
    pool: &Pool,
    user: Pubkey,
    token_program: Pubkey,
    quote: Pubkey,
    program: Pubkey,
) -> Vec<AccountMeta> {
    // Account order follows PumpSwap's buy/sell instruction layout.
    vec![
        AccountMeta::new(pool.address, false),
        AccountMeta::new(user, true),
        AccountMeta::new_readonly(key("ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw"), false),
        AccountMeta::new_readonly(pool.base_mint, false),
        AccountMeta::new_readonly(quote, false),
        AccountMeta::new(ata(user, pool.base_mint, token_program), false),
        AccountMeta::new(ata(user, quote, key(SPL)), false),
        AccountMeta::new(pool.base_vault, false),
        AccountMeta::new(pool.quote_vault, false),
        AccountMeta::new_readonly(token_program, false),
        AccountMeta::new_readonly(key(SPL), false),
        AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        AccountMeta::new(
            pda(program, &[b"user_volume_accumulator", user.as_ref()]),
            false,
        ),
        AccountMeta::new_readonly(key("5PHirr8joyTMp9JMm6nW7hNDVyEYdkzDqazxPD7RaTjx"), false),
        AccountMeta::new(ata(key(BUYBACK), quote, key(SPL)), false),
        AccountMeta::new_readonly(pda(program, &[b"__event_authority"]), false),
        AccountMeta::new_readonly(program, false),
    ]
}
