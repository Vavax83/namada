use std::collections::{HashMap, HashSet};

use criterion::{criterion_group, criterion_main, Criterion};
use namada::core::types::account::AccountPublicKeysMap;
use namada::core::types::address;
use namada::ledger::storage::DB;
use namada::token::{Amount, Transfer};
use namada::tx::Signature;
use namada::vm::wasm::TxCache;
use namada_apps::bench_utils::{
    BenchShell, TX_INIT_PROPOSAL_WASM, TX_REVEAL_PK_WASM, TX_TRANSFER_WASM,
    TX_UPDATE_ACCOUNT_WASM, VP_USER_WASM, WASM_DIR,
};
use namada_apps::wallet::defaults;
use namada_apps::wasm_loader;

// Benchmarks the validation of a single signature on a single `Section` of a
// transaction
fn tx_section_signature_validation(c: &mut Criterion) {
    let shell = BenchShell::default();
    let transfer_data = Transfer {
        source: defaults::albert_address(),
        target: defaults::bertha_address(),
        token: address::nam(),
        amount: Amount::native_whole(500).native_denominated(),
        key: None,
        shielded: None,
    };
    let tx = shell.generate_tx(
        TX_TRANSFER_WASM,
        transfer_data,
        None,
        None,
        vec![&defaults::albert_keypair()],
    );
    let section_hash = tx.header_hash();

    let pkim = AccountPublicKeysMap::from_iter([
        defaults::albert_keypair().to_public()
    ]);

    let multisig = Signature::new(
        vec![section_hash],
        pkim.index_secret_keys(vec![defaults::albert_keypair()]),
        None,
    );

    c.bench_function("tx_section_signature_validation", |b| {
        b.iter(|| {
            multisig
                .verify_signature(
                    &mut HashSet::new(),
                    &pkim,
                    &None,
                    &mut || Ok(()),
                )
                .unwrap()
        })
    });
}

fn compile_wasm(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile_wasm");
    let mut txs: HashMap<&str, Vec<u8>> = HashMap::default();

    for tx in [
        TX_TRANSFER_WASM,
        TX_INIT_PROPOSAL_WASM,
        TX_REVEAL_PK_WASM,
        TX_UPDATE_ACCOUNT_WASM,
        VP_USER_WASM,
    ] {
        let wasm_code = wasm_loader::read_wasm_or_exit(WASM_DIR, tx);
        txs.insert(tx, wasm_code);
    }

    // Test the compilation of a few different transactions
    for (wasm, wasm_code) in txs {
        // Extract the throughput, together with the
        // wall-time, so that we can than invert it to calculate the
        // desired metric (time/byte)
        let len = wasm_code.len() as u64;
        group.throughput(criterion::Throughput::Bytes(len));
        group.bench_function(format!("Wasm: {wasm}, size: {len}"), |b| {
            b.iter_batched_ref(
                || {
                    let mut shell = BenchShell::default();
                    // Re-initialize the tx cache to make sure we are not
                    // reading the precompiled modules from there
                    let tempdir = tempfile::tempdir().unwrap();
                    let path = tempdir.path().canonicalize().unwrap();
                    shell.tx_wasm_cache = TxCache::new(path, 50 * 1024 * 1024);

                    (shell, tempdir)
                },
                |(shell, _tempdir)| {
                    shell
                        .tx_wasm_cache
                        .compile_or_fetch(&wasm_code)
                        .unwrap()
                        .unwrap()
                },
                criterion::BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

fn untrusted_wasm_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("untrusted_wasm_validation");
    let mut txs: HashMap<&str, Vec<u8>> = HashMap::default();

    for tx in [
        TX_TRANSFER_WASM,
        TX_INIT_PROPOSAL_WASM,
        TX_REVEAL_PK_WASM,
        TX_UPDATE_ACCOUNT_WASM,
    ] {
        let wasm_code = wasm_loader::read_wasm_or_exit(WASM_DIR, tx);

        txs.insert(tx, wasm_code);
    }

    // Test the validation of a few different transactions
    for (tx, wasm_code) in txs {
        // Extract the throughput, together with the wall-time, so that we can
        // than invert it to calculate the desired metric (time/byte)
        let len = wasm_code.len() as u64;
        group.throughput(criterion::Throughput::Bytes(len));
        group.bench_function(format!("Tx: {tx}, size: {len}"), |b| {
            b.iter(|| namada::vm::validate_untrusted_wasm(&wasm_code).unwrap())
        });
    }
    group.finish();
}

// Generate some variable-length keys with some hardcoded lengths for the values
fn generate_random_keys_sized() -> Vec<(String, u64)> {
    vec![
        ("bench".to_string(), 1),
        ("bench".to_string(), 1_000),
        ("bench".to_string(), 200_000),
        ("bench/test/key/middle/size".to_string(), 20),
        ("bench/test/key/middle/size".to_string(), 5_000),
        ("bench/test/key/middle/size".to_string(), 5_000_000),
        (
            format!(
                "very/long/{}/bench/test/storage/{}/key/for/benchmark/\
                 purposes/{}",
                defaults::albert_address(),
                defaults::christel_address(),
                defaults::bertha_address()
            ),
            50,
        ),
        (
            format!(
                "very/long/{}/bench/test/storage/{}/key/for/benchmark/\
                 purposes/{}",
                defaults::albert_address(),
                defaults::christel_address(),
                defaults::bertha_address()
            ),
            8_000,
        ),
        (
            format!(
                "very/long/{}/bench/test/storage/{}/key/for/benchmark/\
                 purposes/{}",
                defaults::albert_address(),
                defaults::christel_address(),
                defaults::bertha_address()
            ),
            50_000_000,
        ),
    ]
}

fn write_log_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_log_read");
    let mut shell = BenchShell::default();

    for (key, value_len) in generate_random_keys_sized() {
        let key = namada::core::types::storage::Key::parse(key).unwrap();
        // Extract the throughput, together with the wall-time, so that we can
        // than invert it to calculate the desired metric (time/byte)
        // NOTE: criterion states that the throughput is measured on the
        // processed bytes but in this case we are interested in the input +
        // output bytes, i.e. the combined length of the key and value red, so
        // we set this as the throughput parameter
        let throughput_len = value_len + key.len() as u64;
        group.throughput(criterion::Throughput::Bytes(throughput_len));
        // Generate random bytes for the value and write it to storage
        let value: Vec<u8> = (0..value_len).map(|_| rand::random()).collect();
        shell.wl_storage.write_log.write(&key, value).unwrap();

        group.bench_function(
            format!("key: {key}, bytes: {throughput_len}"),
            |b| {
                b.iter_with_large_drop(|| {
                    shell.wl_storage.write_log.read(&key).0.unwrap()
                })
            },
        );
    }

    group.finish();
}

fn storage_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_read");
    let mut shell = BenchShell::default();

    for (key, value_len) in generate_random_keys_sized() {
        let key = namada::core::types::storage::Key::parse(key).unwrap();
        // Extract the throughput, together with the wall-time, so that we can
        // than invert it to calculate the desired metric (time/byte)
        // NOTE: criterion states that the throughput is measured on the
        // processed bytes but in this case we are interested in the input +
        // output bytes, i.e. the combined length of the key and value red, so
        // we set this as the throughput parameter
        let throughput_len = value_len + key.len() as u64;
        group.throughput(criterion::Throughput::Bytes(throughput_len));
        // Generate random bytes for the value and write it to storage
        let value: Vec<u8> = (0..value_len).map(|_| rand::random()).collect();
        // NOTE: just like for storage writes, we don't have control on when
        // data is actually flushed to disk, so just benchmark the read function
        // without caring if data is actually in memory or on disk
        shell.wl_storage.storage.write(&key, &value).unwrap();

        group.bench_function(
            format!("key: {key}, bytes: {throughput_len}"),
            |b| {
                b.iter_with_large_drop(|| {
                    shell
                        .wl_storage
                        .storage
                        .db
                        .read_subspace_val(&key)
                        .unwrap()
                        .unwrap()
                })
            },
        );
    }

    group.finish();
}

fn write_log_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_log_write");
    let mut shell = BenchShell::default();

    for (key, value_len) in generate_random_keys_sized() {
        let key = namada::core::types::storage::Key::parse(key).unwrap();
        // Extract the throughput, together with the wall-time, so that we can
        // than invert it to calculate the desired metric (time/byte)
        // NOTE: criterion states that the throughput is measured on the
        // processed bytes but in this case we are interested in the input +
        // output bytes, i.e. the combined length of the key and value written,
        // so we set this as the throughput parameter
        let throughput_len = value_len + key.len() as u64;
        group.throughput(criterion::Throughput::Bytes(throughput_len));

        group.bench_function(
            format!("key: {key}, bytes: {throughput_len}"),
            |b| {
                b.iter_batched(
                    || {
                        // Generate random bytes for the value
                        (0..value_len).map(|_| rand::random()).collect()
                    },
                    |value| {
                        shell.wl_storage.write_log.write(&key, value).unwrap()
                    },
                    criterion::BatchSize::SmallInput,
                )
            },
        );
    }

    group.finish();
}

fn storage_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_write");
    let mut shell = BenchShell::default();

    for (key, value_len) in generate_random_keys_sized() {
        let key = namada::core::types::storage::Key::parse(key).unwrap();
        // Extract the throughput, together with the wall-time, so that we can
        // than invert it to calculate the desired metric (time/byte)
        // NOTE: criterion states that the throughput is measured on the
        // processed bytes but in this case we are interested in the input +
        // output bytes, i.e. the combined length of the key and value written,
        // so we set this as the throughput parameter
        let throughput_len = value_len + key.len() as u64;
        group.throughput(criterion::Throughput::Bytes(throughput_len));
        let block_height = shell.wl_storage.storage.block.height;

        group.bench_function(
            format!("key: {key}, bytes: {throughput_len}"),
            |b| {
                b.iter_batched_ref(
                    || {
                        // Generate random bytes for the value
                        (0..value_len).map(|_| rand::random()).collect()
                    },
                    |value: &mut Vec<u8>| {
                        // NOTE: RocksDB actually flushes data to the OS buffer
                        // that will eventually be committed to the actual
                        // storage. We don't really have control on this so we
                        // just benchmark the write operation here without
                        // focusing on the hardware write
                        shell
                            .wl_storage
                            .storage
                            .db
                            .write_subspace_val(block_height, &key, value)
                            .unwrap();
                    },
                    criterion::BatchSize::SmallInput,
                )
            },
        );
    }

    group.finish();
}

criterion_group!(
    host_env,
    tx_section_signature_validation,
    compile_wasm,
    untrusted_wasm_validation,
    write_log_read,
    storage_read,
    write_log_write,
    storage_write,
);
criterion_main!(host_env);
