use super::{ata, key, pda, BUYBACK, FEE_PROGRAM, PUMP, SPL, WSOL};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

/// Build the Pump bonding-curve buy or sell instruction.
pub fn instruction(
    mint: Pubkey,
    user: Pubkey,
    token_program: Pubkey,
    buy: bool,
    amount: u64,
    min_out: u64,
) -> Instruction {
    let program = key(PUMP);
    let quote = key(WSOL);
    let curve = pda(program, &[b"bonding-curve", mint.as_ref()]);
    let data = instruction_data(buy, amount, min_out);

    Instruction {
        program_id: program,
        accounts: accounts(mint, user, token_program, curve, quote, program),
        data,
    }
}

/// Encode the discriminator and arguments for a curve swap.
fn instruction_data(buy: bool, amount: u64, min_out: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(25);
    data.extend_from_slice(if buy {
        &[225, 247, 80, 30, 213, 179, 132, 136]
    } else {
        &[28, 146, 222, 119, 38, 196, 105, 213]
    });
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&min_out.to_le_bytes());
    // OptionBool(true) permits a partial fill when the bonding curve completes.
    if buy {
        data.push(1);
    }
    data
}

/// List the accounts required by the Pump curve instruction.
fn accounts(
    mint: Pubkey,
    user: Pubkey,
    token_program: Pubkey,
    curve: Pubkey,
    quote: Pubkey,
    program: Pubkey,
) -> Vec<AccountMeta> {
    // Account order follows the Pump program's buy/sell instruction layout.
    vec![
        AccountMeta::new_readonly(pda(program, &[b"global"]), false),
        AccountMeta::new_readonly(mint, false),
        AccountMeta::new_readonly(quote, false),
        AccountMeta::new_readonly(token_program, false),
        AccountMeta::new_readonly(key(SPL), false),
        AccountMeta::new(curve, false),
        AccountMeta::new(ata(curve, mint, token_program), false),
        AccountMeta::new(ata(curve, quote, key(SPL)), false),
        AccountMeta::new(user, true),
        AccountMeta::new(ata(user, mint, token_program), false),
        AccountMeta::new(ata(user, quote, key(SPL)), false),
        AccountMeta::new(
            pda(program, &[b"user_volume_accumulator", user.as_ref()]),
            false,
        ),
        AccountMeta::new_readonly(
            pda(key(FEE_PROGRAM), &[b"fee_config", program.as_ref()]),
            false,
        ),
        AccountMeta::new(key(BUYBACK), false),
        AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        AccountMeta::new_readonly(pda(program, &[b"__event_authority"]), false),
        AccountMeta::new_readonly(program, false),
    ]
}
