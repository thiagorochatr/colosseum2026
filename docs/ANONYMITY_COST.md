# Anonymity cost of a clean-set policy

Status: measured offline on 2026-10-12, from mainnet data collected on 2026-07-26. Every number here is printed by one command (see [Reproduce](#reproduce)).

## Question

On Solana, a curator publishes a clean set: the pool's deposits minus those whose depositor its policy blocks. A withdrawal proves membership in the clean set, so the clean set is the withdrawal's anonymity set. A policy therefore costs anonymity twice. It shrinks the set, and it changes which funding sources the remaining members come from, and an observer can partition members by funding source. How much effective anonymity does each kind of policy cost, on real depositors?

## Data, and its limits

- **Population.** The headline sample behind mirror-pool's funding-provenance measurement is `mirror-pool/data/sample-privacycash-run6.json` (run 6 in [MEASUREMENT_LOG.md](../mirror-pool/docs/MEASUREMENT_LOG.md)). It holds the depositors of the live Privacy Cash pool on Solana mainnet, collected on 2026-07-26. The frame was 100 depositors. 17 were excluded as non-wallets, 83 were traced, and 54 of those resolved into 23 funding classes. The other 29 are budget-unresolved: 28 hit the tracer's page cap and 1 its depth limit.
- **Class mix.** One R5 distributor class of 22 members. Three R3 fee-payer clusters of 7, 4 and 2. Four R1 program-funded singletons. Fifteen R4 "busy-unlabelled" singletons. The rules are defined in [PROVENANCE_METHOD.md §4.5](../mirror-pool/docs/PROVENANCE_METHOD.md).
- **Contrast.** `sample-marinade-run8.json` holds Marinade staking depositors: 92 traced, 42 resolved into 33 classes. It fails the method's informativeness gate (fewer than half resolved), so it is used only for contrast.
- **The curator labels are a proxy.** The shipped label set has no exchange or sanctions anchors, so no member carries a CEX label. "Regulated source" is approximated by the structural rules R3 (fee-payer clusters, the pattern the method expects exchanges to show) and R5 (distributors). Nothing here verifies that any class is an exchange. "Untraced" means our tracer ran out of budget. It does not mean the source is unknown in a compliance sense.
- **Small.** 83 members, one pool, one date. The sample treats these depositors as the pool.

## Method

The policies are applied to the measured members, and the method's metrics are recomputed over what each clean set keeps. Classification, `ρ`, effective-k, the unresolved bracket and the resampling interval all come from `mirror-provenance` unchanged. The observer is the method's `A_prov`: chain-only, knows the clean set (it is published) and the labels, and sees the funding class of a withdrawal. Under that observer, a withdrawer hides among the clean-set members of its own class.

**Effective-k** of a clean set is `ρ'·K'`. This is the method's reading of `ρ` as the share of nominal k that survives. It assumes the unresolved members are spread like the resolved ones (missing at random, MAR).

**Retained** is that effective-k divided by the uncurated pool's. Its 95% interval comes from a paired bootstrap: all members are resampled, and both sets are measured on each replicate.

**`ρ'` resampled** is the crate's own `loss_factor_interval`, at its published seed. For a random blocklist, each replicate also redraws which members the list hits.

**Admitted members keep** is the share of their own class-mates that members still in the set keep. It is a geometric mean, given as two readings: unresolved spread like the resolved (MAR), and unresolved apart from every class.

10,000 replicates and 10,000 blocklist draws, with fixed seeds. The metric definitions, adversary and claim rules are in [PROVENANCE_METHOD.md](../mirror-pool/docs/PROVENANCE_METHOD.md) (§1, §2, §5.2, §9).

**P0 reproduces the published run.** The tool prints ρ 0.0955, bracket 0.0350 .. 0.1136, resampled 0.0848 .. 0.1790, 54 resolved of 83. These are the figures in MEASUREMENT_LOG.md and in `mirror analyze --sample data/sample-privacycash-run6.json`, and the tool checks them against `PUBLISHED_HEADLINE`.

## Results

Privacy Cash, run 6. P1 rows are means over blocklist draws, where each depositor is blocked independently with probability p.

| Policy | Kept K' | Classes | ρ' | ρ' resampled 95% | Effective-k | Retained (95%) | Admitted members keep |
|---|---|---|---|---|---|---|---|
| P0 no curation | 83 (100%) | 23 | 0.0955 | 0.0848 .. 0.1790 | 7.92 | 1 | 1 |
| P1 random blocklist, 1% | 82.2 (99%) | 22.8 | 0.0958 | 0.0841 .. 0.1819 | 7.88 | 0.99 (0.95 .. 1.04) | 0.99 |
| P1 random blocklist, 5% | 78.9 (95%) | 22.1 | 0.0975 | 0.0856 .. 0.1855 | 7.69 | 0.97 (0.87 .. 1.06) | 0.96 .. 0.97 |
| P1 random blocklist, 10% | 74.7 (90%) | 21.1 | 0.0999 | 0.0857 .. 0.1932 | 7.46 | 0.94 (0.79 .. 1.06) | 0.93 .. 0.94 |
| P1 random blocklist, 25% | 62.2 (75%) | 18.2 | 0.1082 | 0.0890 .. 0.2236 | 6.72 | 0.85 (0.62 .. 1.02) | 0.81 .. 0.84 |
| P2a block untraced funding | 54 (65%) | 23 | 0.0955 | 0.0848 .. 0.1790 | 5.15 | 0.65 (0.54 .. 0.75) | 0.65 .. 1 |
| P2b block unattributed (untraced + R4) | 39 (47%) | 8 | 0.2484 | 0.1980 .. 0.3922 | 9.69 | 1.22 (0.82 .. 1.35) | 0.65 .. 1 |
| P2c block one source, the largest class (R5, 22) | 61 (73%) | 22 | 0.0594 | 0.0646 .. 0.1231 | 3.62 | 0.46 (0.35 .. 0.76) | 1 |
| P3a allow only R3 clusters | 13 (16%) | 3 | 0.3738 | 0.3353 .. 0.5826 | 4.86 | 0.61 (0.24 .. 0.92) | 0.65 .. 1 |
| P3b allow only R3 clusters + R5 | 35 (42%) | 4 | 0.3587 | 0.2953 .. 0.5022 | 12.56 | 1.59 (0.98 .. 1.68) | 0.65 .. 1 |

The other readings of the unresolved members are below. The bracket reads them as singletons and as one class (§2.6). Miller–Madow corrects plug-in entropy's bias, as a sensitivity check (§2.6a); the crate publishes plug-in.

| Policy | Effective-k bracket | Retained, Miller–Madow |
|---|---|---|
| P0 | 2.91 .. 9.43 | 1 |
| P1 25% | 2.61 .. 7.67 | 0.84 |
| P2a | 5.15 (no unresolved left) | 0.65 |
| P2b | 9.69 (no unresolved left) | 1.37 |
| P2c | 1.40 .. 6.94 | 0.40 |
| P3a | 4.86 (no unresolved left) | 0.70 |
| P3b | 12.56 (no unresolved left) | 1.86 |

Contrast, Marinade run 8 (not informative at P0): retained 1.00 / 0.99 / 0.97 / 0.94 for P1 at 1 / 5 / 10 / 25%, 0.46 for P2a, 0.60 (0.35 .. 0.75) for P2b, 0.95 (0.86 .. 1.04) for P2c (largest class: an R3 cluster of 4), and 0.70 for P3a and P3b. The full table is in the tool's output.

## Findings

- **A random blocklist costs about its own size.** Blocking p of the depositors keeps about 1 − p of effective-k (0.99, 0.97, 0.94, 0.85), and every admitted member loses about p of its class-mates. The result is slightly better than 1 − p because 35% of resolved members are already alone in their class and have no cover to lose. That is a small-sample effect, and in a large pool the cost should tend to p. Every interval contains 1 − p. This is the deployed curator's rule ("exclude every deposit whose depositor is on the blocklist"), provided the blocked depositors are unrelated to funding source.
- **Policies keyed on funding source shrink K far more than effective-k, and ρ rises.** They remove whole classes, so a member still admitted keeps all its class-mates under the "unresolved apart" reading, and loses the untraced share (35%) under MAR. Blocking unattributed funding (P2b) removes 53% of depositors, yet effective-k is 9.69 against 7.92, retained 1.22 (0.82 .. 1.35): not distinguishable from unchanged. Allowing only R3 and R5 sources (P3b) keeps 42% of depositors, with retention 1.59 (0.98 .. 1.68). ρ goes from 0.0955 to 0.25–0.37 because the remaining set is more homogeneous.
- **That rise is composition, not protection.** It means the poorly hidden members were removed. Nobody still admitted is better hidden than before. The reverse case shows it: blocking the single largest source (P2c, 27% of depositors) removes the biggest crowd. Effective-k falls to 0.46 of baseline (0.35 .. 0.76), worse than a random 25% block (0.85), even though no admitted member lost a class-mate. A blocklist that is correlated with funding, such as every wallet one exploiter funded, behaves like P2c rather than P1.
- **The sign depends on the pool, not the policy.** In the Marinade contrast there is no large crowd. Blocking the largest source costs almost nothing (0.95). Blocking unattributed funding costs 40% (0.60) instead of holding steady. A provenance policy's cost has to be measured on each pool's own class distribution.
- **Excluding what the tracer could not trace (P2a) costs between nothing and 35% of effective-k,** depending on how the unresolved are read. It is also the one case where the figure is exact for the set, because no unresolved members are left.

## What this does not show

- **That any class is an exchange or risky.** The labels are structural proxies, and the shipped anchor set is empty. A real travel-rule allowlist cannot be measured on this data.
- **Protection against a finer observer.** Curator and observer use the same labels here, so "admitted members keep 1" for a whole-class policy is an identity, not a finding (§9.1). An observer with finer labels than the curator would split classes the curator kept whole. Measuring that needs a held-out labeller, and none was run.
- **That the 22-member class is 22 people.** The method treats members with the same funder as distinct principals (§5.2). In aggregate, those 22 were funded on 14 different days over ten months, with 22 different amounts, from an address with at least 24,000 transactions. That looks like a service rather than one person splitting funds, but it is not proof. If it is one principal, P2b's and P3b's favourable rows and P2c's cost are all overstated.
- **Large-pool values.** Effective-k at small k understates the steady-state loss (§2.4), and plug-in ρ is biased high (§2.6a). The ratios are within-sample. The Miller–Madow sensitivity points the same way on every row.
- **Other channels.** Timing, amount, fee payer and the network are excluded, so real effective-k is at most these figures (§1.3). A clean set limited to one kind of source also tells the outside world what kind of source every withdrawer has (§9.3).
- **What the excluded lose.** They cannot withdraw privately at all; Solana has no ragequit yet ([LIMITS_AND_FAQ.md](LIMITS_AND_FAQ.md)). These figures describe only the depositors who remain.
- **Anything about Privacy Cash's own screening, or about this project's pools,** which have no real depositors.

## Reproduce

From the repository root:

```
cargo run --release --offline --manifest-path tools/curation-cost/Cargo.toml
```

It is offline, deterministic and fixed-seed, and it prints both tables with the P0 check. The code is in [`tools/curation-cost`](../tools/curation-cost). To cross-check P0 against the crate's own command: `cd mirror-pool && cargo run -p mirror-cli -- analyze --sample data/sample-privacycash-run6.json`.
