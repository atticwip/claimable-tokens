#![cfg(feature = "test-bpf")]

use solana_program::{pubkey::Pubkey, program_pack::Pack, system_instruction};
use claimable_tokens::*;
use solana_program_test::*;
use solana_sdk::{
    account::Account,
    signature::{Keypair, Signer},
    transaction::Transaction,
    transport::TransportError,
};
use rand::{thread_rng, Rng};
use secp256k1::{PublicKey, SecretKey};
use sha3::{Digest, Keccak256};
use borsh::de::BorshDeserialize;

pub fn program_test() -> ProgramTest {
    ProgramTest::new(
        "claimable_tokens",
        id(),
        processor!(processor::Processor::process_instruction),
    )
}

pub async fn get_account(program_context: &mut ProgramTestContext, pubkey: &Pubkey) -> Account {
    program_context
        .banks_client
        .get_account(*pubkey)
        .await
        .expect("account not found")
        .expect("account empty")
}

fn construct_eth_address(
    pubkey: &PublicKey,
) -> [u8; state::ETH_ADDRESS_SIZE] {
    let mut addr = [0u8; state::ETH_ADDRESS_SIZE];
    addr.copy_from_slice(&Keccak256::digest(&pubkey.serialize()[1..])[12..]);
    assert_eq!(addr.len(), state::ETH_ADDRESS_SIZE);
    addr
}

async fn create_account(
    program_context: &mut ProgramTestContext,
    account_to_create: &Keypair,
    lamports: u64,
    space: u64,
    owner: &Pubkey,
) -> Result<(), TransportError> {
    let mut transaction = Transaction::new_with_payer(
        &[system_instruction::create_account(
            &program_context.payer.pubkey(),
            &account_to_create.pubkey(),
            lamports,
            space,
            owner,
        )],
        Some(&program_context.payer.pubkey()),
    );
    transaction.sign(
        &[&program_context.payer, account_to_create],
        program_context.last_blockhash,
    );
    program_context
        .banks_client
        .process_transaction(transaction)
        .await?;
    Ok(())
}

async fn create_mint(
    program_context: &mut ProgramTestContext,
    mint_account: &Keypair,
    mint_rent: u64,
    authority: &Pubkey,
) -> Result<(), TransportError> {
    let instructions = vec![
        system_instruction::create_account(
            &program_context.payer.pubkey(),
            &mint_account.pubkey(),
            mint_rent,
            spl_token::state::Mint::LEN as u64,
            &spl_token::id(),
        ),
        spl_token::instruction::initialize_mint(
            &spl_token::id(),
            &mint_account.pubkey(),
            authority,
            None,
            0,
        )
        .unwrap(),
    ];

    let mut transaction =
        Transaction::new_with_payer(&instructions, Some(&program_context.payer.pubkey()));

    transaction.sign(
        &[&program_context.payer, mint_account],
        program_context.last_blockhash,
    );
    program_context
        .banks_client
        .process_transaction(transaction)
        .await?;
    Ok(())
}

async fn init_user_bank(
    program_context: &mut ProgramTestContext,
    bank: &Keypair,
    mint: &Pubkey,
    base_acc: &Pubkey,
    acc_to_create: &Pubkey,
    eth_address: [u8; state::ETH_ADDRESS_SIZE],
) -> Result<(), TransportError> {
    let mut transaction = Transaction::new_with_payer(
        &[instruction::init(
            &id(),
            &bank.pubkey(),
            &program_context.payer.pubkey(),
            mint,
            base_acc,
            acc_to_create,
            eth_address,
        ).unwrap()],
        Some(&program_context.payer.pubkey()));
    
    transaction.sign(&[&program_context.payer, bank], program_context.last_blockhash);
    program_context.banks_client.process_transaction(transaction).await
}

#[tokio::test]
async fn test_init_instruction() {
    let mut program_context = program_test().start_with_context().await;
    let rent = program_context.banks_client.get_rent().await.unwrap();

    let mut rng = thread_rng();
    let key: [u8; 32] = rng.gen();
    let priv_key = SecretKey::parse(&key).unwrap();
    let secp_pubkey = PublicKey::from_secret_key(&priv_key);
    let eth_address = construct_eth_address(&secp_pubkey);

    let bank_account = Keypair::new();
    create_account(&mut program_context, &bank_account, rent.minimum_balance(state::UserBank::LEN), state::UserBank::LEN as u64, &id()).await.unwrap();

    let mint_account = Keypair::new();
    let mint_authority = Keypair::new();
    create_mint(&mut program_context, &mint_account, rent.minimum_balance(spl_token::state::Mint::LEN), &mint_authority.pubkey()).await.unwrap();

    let (base_acc, _) =
            Pubkey::find_program_address(&[&mint_account.pubkey().to_bytes()[..32], &eth_address], &id());
    let address_to_create =
        Pubkey::create_with_seed(&base_acc, processor::Processor::TOKEN_ACC_SEED, &spl_token::id()).unwrap();

    init_user_bank(&mut program_context, &bank_account, &mint_account.pubkey(), &base_acc, &address_to_create, eth_address).await.unwrap();

    let bank_account_data = get_account(&mut program_context, &bank_account.pubkey()).await;
    let bank_account = state::UserBank::try_from_slice(&bank_account_data.data.as_slice()).unwrap();

    assert!(bank_account.is_initialized());
    assert_eq!(bank_account.eth_address, eth_address);
    assert_eq!(bank_account.token_account, address_to_create);

    let token_account_data = get_account(&mut program_context, &address_to_create).await;
    // check that token account is initialized
    let token_account = spl_token::state::Account::unpack(&token_account_data.data.as_slice()).unwrap();

    assert_eq!(token_account.mint, mint_account.pubkey());
}
