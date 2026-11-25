---
sidebar_position: 1
---
# Hardware Requirements

Pathfinder's hardware requirements depend greatly on your use case. Below are two profiles for running Pathfinder.

## Profiles

### Low to medium RPC traffic

- **CPU**: 4 vCPU
- **RAM**: 8 GB
- **Storage**: 1 TB SSD

:::info

On machines with around 8 GM of RAM, Pathfinder may refuse to start with the JSON-RPC server enabled until you set the compiler concurrency limit yourself. See [RAM](#ram) below.

:::

### Heavy RPC traffic

- **CPU**: 4 vCPU
- **RAM**: 16 GB
- **Storage**: 1 TB SSD

Each hardware component has a different impact on Pathfinder's performance.

## CPU

CPU is what limits JSON-RPC throughput. Use is low while the node only follows the chain, and grows with the traffic you send it. Methods that execute contract code, such as `starknet_estimateFee`, `starknet_simulateTransactions` and `starknet_traceTransaction`, cost much more CPU than plain reads.

:::note

The counts above are vCPUs. On x86 machines a vCPU is usually one hardware thread, so four vCPUs share two physical cores. On Arm machines a vCPU is usually a whole core.

:::

## RAM

Pathfinder's own memory use is modest. Most of a machine's RAM ends up caching database pages, which is what keeps reads fast.

The one thing that needs a potentially lot of memory at once is compiling Sierra contract classes to CASM for JSON-RPC calls. The node limits how many of these run in parallel based on the memory it can see, and the `--rpc.compiler.concurrency-limit` option documents the formula.

:::info

On machines around 8 GB that limit can come out as zero, because the memory the operating system reports is lower than the nominal size of the machine. The node then refuses to start with the JSON-RPC server enabled and prints what you can change. Setting the limit yourself gets you past it:

```bash
--rpc.compiler.concurrency-limit=1
```

Or as an environment variable:

```bash
PATHFINDER_RPC_COMPILER_CONCURRENCY_LIMIT=1
```

:::

## Storage

Storage limits how much history the node can keep. Pathfinder's reads are usually small and scattered across the whole database, which is the access pattern a spinning disk handles worst. We recommend using a high-speed SSD drive.

The size of the database depends on the history you keep. The [snapshot download page](https://rpc.pathfinder.swmansion.com/snapshots/latest) lists the current size of each network's database. [State Trie Pruning](./configuration.md#state-trie-pruning) and [Blockchain Pruning](./configuration.md#blockchain-pruning) are the two options that directly impact them. By default, Pathfinder keeps the full blockchain history but prunes state tries, so a fresh database holds tries for the last 21 blocks. Every block, transaction, receipt and event stays queryable, while storage proofs are available for recent blocks only.

Pick the mode before you sync. It is fixed when the database is created, so changing it later means re-syncing or starting from a different snapshot.

:::note

Keeping the full history of state tries, so the node can serve storage proofs for any block rather than only recent ones, is a separate setting. It stores every historical trie node instead of discarding old ones, so the database grows much faster and needs more disk space than either profile above allows. See [State Trie Pruning](./configuration.md#state-trie-pruning).

:::
