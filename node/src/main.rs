use node::{blockchain::Blockchain, mempool::Mempool, state::State};

fn main() {
    let state = State::new();
    let mempool = Mempool::new(10_000);
    let chain = Blockchain::new();

    println!(
        "Trilogicon node scaffold ready | accounts={} mempool_capacity={} chain_height={}",
        state.account_count(),
        mempool.capacity(),
        chain.height()
    );
}
