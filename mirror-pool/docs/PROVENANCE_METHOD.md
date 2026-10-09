# Funding-Provenance Effective-k — measurement specification

Status: specification, v1 — and `crates/mirror-provenance` was built against it.

Read it as the method rather than as a description of the code. Most of it
shipped; three things did not, and each is marked where it appears rather than
quietly dropped:

- **`verify-labels`** (§5.4) was specified to re-check every curated anchor
  against the chain. The anchor set exists in `classify.rs`; the command that
  falsifies it does not, so Tier-2 labels rest on their cited sources alone.
- **The multi-file artifact layout** (§7.3) became one JSON document per run.
  What shipped is described there.
- **The L0–L3 label-resolution ladder** (§5.1, §5.6). The classification rules
  R1–R5 shipped and are what produce every published label. Reporting the *same*
  measurement at four label granularities, and the monotonic bracket that would
  come out of it, did not: the crate has no notion of a resolution rung, so the
  published headline is a single figure at the resolution the R-rules give,
  neither claimed nor claimable as "L2".
- **The `n = 1,100` baseline frame** (§3) was not collected. The published runs
  are the smaller ones in `docs/MEASUREMENT_LOG.md`, and what that costs the
  estimate is stated there rather than here.

Every empirical number below names its source. Numbers tagged **[M 2026-07-25]** were
measured directly against Solana mainnet on that date (probe method stated inline);
numbers tagged **[V]** were verified against a primary document. Everything else is
design and must not be reported as a finding.

---

## 0. Purpose and scope

A privacy pool advertises `k` members. On a public ledger an observer can partition
those members into **funding-provenance classes** — where each member's capital came
from. If the partition is informative, the anonymity a member actually receives is
below `k`.

This document specifies how we measure that gap: the sampling frame, the tracing
algorithm, the metric, the label resolutions, the failure accounting, and the on-disk
artifacts that let a third party reproduce every published number without network
access.

It also specifies, in §9, what our own protocol may and may not claim about reducing
the gap — including which framings would be tautological if measured naively.

**This crate measures. It makes no privacy claim by itself.** A number produced by
this tool describes one channel under one stated adversary at one stated label
resolution, at one pinned slot. It is not a security proof and must never be
presented as one.

### 0.1 Conventions

| symbol | meaning |
|---|---|
| `S` | snapshot slot. Nothing with `slot > S` is visible to any stage. |
| `K` | number of members in the measured anonymity set (after §5.2 collapse) |
| `m` | number of provenance classes observed |
| `n_c` | size of class `c`; `Σ_c n_c = K` |
| `X` | random variable: which member performed the action |
| `C` | random variable: the observed provenance class, `C = λ(X)` |
| `λ_L` | labelling function at resolution `L` (§5) |

---

## 1. Adversary model

Every reported number is meaningless without this section printed beside it. The tool
MUST emit the adversary identifier and label resolution in the same object as the
metric.

### 1.1 `A_prov` — passive, global, retrospective, chain-only

**Knows:**

- The finalized Solana ledger up to snapshot slot `S`.
- The complete member set `A = {x_1 … x_K}` of the measured round. Membership is
  public by construction: a deposit is an on-chain event.
- The labelling function `λ_L`, which we publish and ship. The adversary is granted
  our labels exactly, no more.

**Observes:**

- One action `a`, known to have been produced by exactly one member.
- An action-side class `μ(a) ∈ Labels ∪ {⊥}`, computed by applying the *same* `λ_L`
  to the action's funding trace.

**Inference rule:** eliminate every `x_i` with `λ_L(x_i) ≠ μ(a)`; uniform posterior
over the survivors. Prior over `X` is uniform on `A`.

**Explicitly denied.** The adversary has none of:

- off-chain identity (KYC records, exchange cooperation, subpoena);
- network-layer observation (gossip, mempool, RPC-timing correlation);
- timing correlation between deposit and action;
- amount correlation between deposit and action;
- behavioural fingerprints (wallet software, compute-budget settings, fee patterns);
- active Sybil deposits into the measured pool;
- compromise of any party.

Those are separate channels. Folding any of them in inflates the number and destroys
comparability with the published Ethereum results we benchmark against (§10.2).

### 1.2 The two-sidedness rule

> A partition is admissible only if the adversary can compute the same label on the
> **member side** and on the **action side**.

This rule is load-bearing, and it is the single design decision the rest of the
method is built around.

Entropy over a partition decreases monotonically under refinement. Therefore **finer
labels always yield a lower effective-k**, and a *weaker* tracer that fails to merge
members reports a *worse* number than a strong one. A metric with that property is not
measuring the pool; it is measuring the tracer's budget.

Raw-address partitioning violates two-sidedness: "funded by Binance hot wallet #7" is
not observable of an action whose trace reaches a *different* Binance hot wallet. The
adversary cannot exploit a distinction they cannot match on both sides. The headline
resolution is therefore the finest **two-sided** one — entity level, not address level
(§5).

Because monotonicity runs the other way from intuition, the tool MUST NOT report a
single scalar. It reports the ladder (§5.5) and the bracket (§2.6).

### 1.3 What the number proves

**Proves.** Under `A_prov` at resolution `L`, at slot `S`, the measured set's
advertised `k` overstates the adversary's residual uncertainty by a factor of
`2^{H_L(C)}`.

**Does not prove.**

1. That any individual was deanonymized. We attempt no linkage and name no member.
   The tool MUST NOT emit member addresses in any published artifact.
2. That the pool is broken. One channel, one adversary, one slot.
3. Anything composable. Effective-k values for different channels do not multiply or
   add. The joint is bounded above by the minimum; computing it requires the joint
   partition.
4. A durable protocol property. It is a property of the measured set at slot `S`.
5. A bound on real-world anonymity. A real adversary holds the denied channels, so
   **real effective-k ≤ ours** at fixed `L`.

**The one monotonicity we may claim:** adding channels only refines the partition, so
our number is an **upper bound on privacy against any strictly stronger adversary at
the same label resolution**. It is *not* a bound in the label-resolution direction.
State both halves. Stating only the first is the common error, and it inverts the
claim: it lets a number measured at coarse labels be read as a guarantee against
an adversary holding finer ones.

---

## 2. Metrics

### 2.1 The core identity

`C` is a deterministic function of `X` (each member belongs to exactly one class), so
`H(C|X) = 0` and the chain rule collapses:

```
log2 K  =  H(C)  +  H(X|C)
```

which gives the definition used throughout:

```
H(X|C) = Σ_c (n_c/K) · log2(n_c)            residual anonymity, in bits
eff_k_shannon = 2^{H(X|C)} = K / 2^{H(C)}   residual anonymity, in members
```

`2^{H(C)}` is the *effective number of classes*. **Effective-k is nominal k divided by
the effective number of classes.**

Two identities worth surfacing in output, because they are exact and quotable:

```
Shannon leakage      I(X;C) = H(C)          bits, exactly
min-entropy leakage  = log2 m               bits, exactly   (Smith 2009, Thm. 1)
```

> **Implementation warning.** `2^{H(C)}` — entropy over the *class-size distribution* —
> is the folklore formula and it is inverted: it is maximised at total deanonymization.
> It is the **leakage**, not the anonymity. Do not compute it as the headline. A unit
> test MUST pin `H(C) + H(X|C) == log2 K`.

### 2.2 Attribution (the widely-repeated version is wrong)

Serjantov & Danezis, *Towards an Information Theoretic Metric for Anonymity*, PET 2002,
LNCS 2482, pp. 41–53. **[V]** Definition 2, verbatim:

> "We define the **effective size S** of an r anonymity probability distribution U to be
> equal to the entropy of the distribution. In other words `S = − Σ_{u∈Ψ} p_u log2(p_u)`
> where `p_u = U(u, r)`."

So `S` is **in bits**, and `p_u` ranges over **users**, not classes. The paper does
convert to a member count in its own §5 worked examples ("equivalent to a threshold mix
with `N = 2^7.127 ≈ 140` inputs") **[V]**, but that is an interpretation, not
Definition 2. The formal definition of `A = 2^{H(P)}` as a *quantity* is Andersson &
Lundin, *On the Fundamentals of Anonymity Metrics*, IFIP AICT 262, 2008, Def. 1
("scaled anonymity set size"), who explicitly dispute the folklore attribution.

**Required wording in any write-up:**

> the effective anonymity set size, i.e. `2^H` where `H` is the Serjantov–Danezis
> entropy metric [S&D 2002, Def. 2]; the exponentiated form is due to Andersson &
> Lundin [2008, Def. 1] and is standard in the mixnet literature [Shmatikov & Wang,
> WPES 2006].

Do **not** write "Serjantov and Danezis define the effective anonymity set size as
`2^H`."

For each observation `C = c` the posterior is uniform on class `c`, whose S&D effective
size is `log2 n_c` bits. `H(X|C)` is therefore the **expectation of the S&D effective
size over the adversary's observation**. That sentence is the whole justification and
it is unattackable; use it.

### 2.3 The reporting ladder

Report all of these, always. Never a scalar.

| # | quantity | formula | interpretation |
|---|---|---|---|
| 1 | **loss factor `ρ`** | `ρ = 2^{−H(C)}` | **headline.** k-independent fraction of nominal k that survives |
| 2 | Shannon effective-k | `2^{H(X\|C)} = Π_c n_c^{n_c/K}` | membership-weighted **geometric** mean of class sizes |
| 3 | Shannon leakage | `H(C)` bits | exact |
| 4 | min-entropy effective-k | `K/m` | Bayes vulnerability; unweighted **arithmetic** mean of class sizes |
| 5 | min-entropy leakage | `log2 m` bits | exact |
| 6 | worst-case class | `min_c n_c` | **must be reported with its null distribution** (§2.5) |
| 7 | class-size CCDF | `P(my class ≤ t)`, `t ∈ {1,2,4,8,16,…}` | the distribution, not a summary |
| 8 | guessing entropy | `G(X\|C) = (1/K)·Σ_c n_c(n_c+1)/2` | expected guesses; baseline `(K+1)/2` |
| 9 | Good–Turing coverage | `Ĉ = 1 − f₁/K` | fraction of class mass observed |
| 10 | Chao1 richness | `m + f₁(f₁−1)/(2(f₂+1))` | saturation diagnostic |

`f₁`, `f₂` = number of classes of size exactly 1, 2.

**Ordering invariant** (verified over 200,000 random partitions, zero violations —
assert it in a property test):

```
min_c n_c  ≤  K/m  ≤  Π_c n_c^{n_c/K}  ≤  K
worst         min-ent    Shannon           advertised
```

Report min-entropy alongside Shannon because Shannon averages and hides the tail. The
canonical counterexample: uniform over 20 users and a 101-user distribution where the
true sender has p = 0.5 both give `H = 4.3219` bits, but the adversary succeeds one
time in twenty in the first case and one time in two in the second (Tóth, Hornák &
Vajda, NordSec 2004).

Guessing entropy is included because it is what the canonical "advertised vs effective"
paper uses — Möser et al., *An Empirical Analysis of Traceability in the Monero
Blockchain*, PoPETs 2018(3), define effective anonymity set size as `1 + 2·Ge`.
**Indexing trap:** their `Ge = Σ_{0≤i≤M} i·p_i` starts at `i = 0` (expected *wrong*
guesses), so `Ge = G_Massey − 1`. Using 1-indexed Massey gives `M+3` where the paper
says `M+1`. Pin this in a test.

Do **not** report Díaz et al.'s normalised degree `d = H(X)/log2 N` as a headline: it
reports ≈1.0 for a large but badly-partitioned pool, which is the opposite of what we
are measuring. Secondary metric only.

### 2.4 `ρ` is the headline, not effective-k

For a fixed class prior, `eff_k(k)/k` decreases toward `2^{−H(C)}` from above. Measured
by simulation over a Zipf-ish 60-class prior **[M 2026-07-25, 400 resamples per k]**:

| nominal k | 8 | 16 | 32 | 64 | 128 | 256 | 512 | 1024 | 4096 |
|---|---|---|---|---|---|---|---|---|---|
| median `eff_k/k` | 0.177 | 0.118 | 0.085 | 0.065 | 0.055 | 0.049 | 0.046 | 0.045 | 0.043 |

(asymptote `2^{−H(C)} = 0.0431`)

**A measurement at small k systematically understates the steady-state loss and cannot
be extrapolated to a large pool.** `ρ` is k-independent, estimable with CIs, and
comparable across pools of different sizes. Publish `ρ` plus the `eff_k(k)` curve for
`k ∈ {8,16,32,64,128,256,512,1024}` with 5th/95th percentiles from `B = 10,000`
resamples of the measured class prior.

### 2.5 Worst case must carry its null

In the same simulation, `P(min_c n_c = 1) ≈ 1.00` for every `k ≤ 512` and 0.93 at
`k = 1024` **[M 2026-07-25]**. Under any heavy-tailed provenance prior *somebody is
always alone*. "Worst case is 1" is therefore not a finding about a pool; reported
bare, it is theatre.

The tool MUST emit `worst_case_observed` together with `worst_case_null_p50` and
`worst_case_null_p05` computed by resampling the fitted class prior at the same `k`.

### 2.6 The unresolved bracket

Members whose trace terminates without a class are **not** a class. Merging them
assumes they are identical (charitable to the pool, raises k); splitting them into
singletons assumes they are all distinct (charitable to the adversary, lowers k).
Neither is known.

Emit three values:

- `eff_k_upper` — unresolved merged into one class;
- `eff_k_lower` — unresolved split into singletons;
- `eff_k_imputed` — `B = 10,000` draws imputing each unresolved member's class from the
  class distribution measured on the **resolved** members. State the missing-at-random
  assumption explicitly and emit an MNAR sensitivity sweep (impute instead from the
  tail of the distribution) alongside.

A point estimate without this bracket is not publishable output.

### 2.6a Sampling error, which the bracket does not cover

The bracket answers *what if the unresolved members had been something else*. It
says nothing about the other uncertainty, which is that the members measured are
a **draw** from a much larger population. Both are needed and neither substitutes
for the other:

| | question | shape |
|---|---|---|
| bracket | what if the unresolved had landed differently? | exact bound, computed |
| bootstrap | how much of `ρ` is which depositors we drew? | percentile interval, resampled |

**The estimator.** Resample the resolved members with replacement, `n` draws for
a sample of `n`, recompute `ρ` on the resampled class tally, repeat `B = 10,000`
times, and take the 2.5th and 97.5th percentiles. A class that no resampled
member landed in is **absent** from that replicate, not a class of size zero —
which is the whole reason the interval is asymmetric under a heavy tail:
resampling merges singletons, and fewer classes means higher `ρ`.

**Reproducibility.** The generator is a splitmix64 implemented in-tree, seeded
from a published constant, because `analyze` is a pure offline pass whose output
a third party must be able to reproduce byte for byte. An external RNG makes that
promise depend on a dependency's version.

#### The bias this does *not* remove, stated because it runs the wrong way for us

Plug-in entropy — estimating `H(C)` by counting — is the maximum-likelihood
estimator, and it is **biased downward** when classes are many and members are
few. That is the regime every run in this project operates in.

Since `ρ = 2^{−H(C)}`, understating `H(C)` means **overstating `ρ`**. So:

> Every `ρ` published here is biased **high**. The true loss factor is plausibly
> smaller — the pools plausibly leak *less* than we report.

A bootstrap resamples the same estimator, so its interval is centred on the
biased value: it measures the estimator's spread, not its distance from the
truth. Correcting this needs Miller–Madow or a coverage-adjusted estimator, and
neither is implemented here.

It is stated this loudly for two reasons. First, it is the direction that makes
our own headline look worse rather than better, and a bias disclosed only when it
flatters is not a disclosure. Second, it is what makes **comparison** valid where
individual numbers are shaky: the bias falls in the same direction on every
population measured the same way, so a difference between two of them survives a
bias that neither absolute number does.

#### Comparing two populations

`ρ` is the headline because it is independent of `k` and therefore comparable
across pools. Exercising that means bootstrapping the **difference**, resampling
each population independently within a replicate, and reporting the interval on
`ρ(A) − ρ(B)`.

**If that interval contains zero, the two populations are not distinguishable at
those sample sizes, and neither may be called the more concentrated.** Subtracting
two point estimates and reporting the sign is the error this exists to prevent:
two samples of the same underlying shape always differ by *something*.

The unit test that matters here is not that a stark difference separates — that
is easy. It is that two identically shaped populations do **not**.

### 2.6b Testing the assumption the bracket rests on

The unresolved bracket bounds what the unresolved members *could* have been. Every
number computed from the resolved ones alone additionally assumes those resolved
members are a **fair draw of the classes present** — that resolvability is
independent of provenance class.

That assumption is usually stated and left there. It is testable.

**The design.** Collect one frame at two budgets. Partition the larger run's
resolved members into

- those the smaller budget also reached — *cheap to trace*, and
- those only the larger budget reached — *expensive to trace*,

and bootstrap the difference in `ρ` between the two groups.

**Separation is the bad outcome.** If expensive-to-trace members carry a different
class distribution from cheap ones, the unresolved are not missing at random, the
resolved subset is biased toward whichever classes happen to be cheap, and no
amount of extra budget repairs it — more budget moves the boundary without
removing it. Every cross-population comparison would then be uninterpretable.

**Measured, both populations, on the runs in `MEASUREMENT_LOG.md`:**

| population | cheap | expensive | difference, 95% |
|---|---|---|---|
| staking control | 16 members, ρ 0.0743 | 22 members, ρ 0.0585 | −0.0144 … +0.0786 |
| privacy pool | 39 members, ρ 0.1179 | 15 members, ρ 0.1250 | −0.1507 … +0.0704 |

Neither separates. At the margin these budgets test, being resolvable is not
picking out particular provenance classes.

**What that licenses, exactly.** It is evidence that the members *just beyond* a
cheaper budget resemble those inside it. It says nothing about members beyond the
larger budget, where a heavier tail could still sit, and it is a statement about
these two populations at these budgets rather than a general law. It is reported
as evidence, never as the assumption being discharged.

### 2.7 Reference vectors for unit tests

Exact values, computed at full double precision. Assert to 1e-9.

**Vector A** — `sizes = [50, 20, 10, 10, 5, 3, 1, 1]`, `K = 100`, `m = 8`

```
H(C)              = 2.1295115772  bits
H(X|C)            = 4.5143446126  bits
H(C) + H(X|C)     = 6.6438561898  == log2(100)
eff_k_shannon     = 22.8535219812
eff_k_minent      = 12.5
minent_leakage    = 3.0           bits   (= log2 8)
worst_case        = 1
G(X|C)            = 16.18                (uniform baseline 50.5)
good_turing_C     = 0.98                 (f1=2, f2=0)
chao1             = 9.0
```

**Vector B** — `sizes = [19, 1×11]`, `K = 30`, `m = 12`

```
H(C)              = 2.2165365038  bits
H(X|C)            = 2.6903540918  bits
eff_k_shannon     = 6.4547181110
eff_k_minent      = 2.5
minent_leakage    = 3.5849625007  bits
worst_case        = 1
G(X|C)            = 6.7                  (uniform baseline 15.5)
good_turing_C     = 0.6333333333         (f1=11, f2=0)
chao1             = 67.0
```

**Vector C** — `sizes = [1×30]` (total deanonymization): `H(X|C) = 0`,
`eff_k_shannon = 1.0`, `eff_k_minent = 1.0`, `worst = 1`, `good_turing_C = 0.0`.

**Vector D** — `sizes = [30]` (nothing partitioned): `H(C) = 0`,
`eff_k_shannon = 30.0`, `eff_k_minent = 30.0`, `worst = 30`, `G = 15.5`.

**Vector E** — `sizes = [4, 4]`: `H(C) = 1.0`, `H(X|C) = 2.0`, `eff_k_shannon = 4.0`,
`eff_k_minent = 4.0`.

**Negative control test (mandatory).** Inject a partial leak into a synthetic uniform
set and assert the metric drops below nominal. A metric that has never produced a bad
number for our own system has not been validated. See §9.0.

---

## 3. Sampling design

### 3.1 Two populations, two claims — never conflate

**P1 — pool census.** All distinct depositors of pool `P` with `S₀ ≤ slot ≤ S`. If the
population is enumerable, **census it**; there is then zero sampling error and §3.4 does
not apply.

> **Failure mode.** Sampling *n* depositors from a pool's most recent signatures and
> then setting *advertised k = n* is a category error that is easy to commit and hard
> to see afterwards: advertised k is the pool's member count, and the sample size is a
> property of the budget. Reporting the second as the first makes the anonymity claim
> a restatement of how long the collector ran.
>
> The scale involved, measured **[M 2026-07-25]**: an active pool emits ~1,000
> signatures per 1.2 days, so a 300-signature scan covers **the most recent ≈10 hours**
> of pool activity. A 90-day census of the same pool is ≈75 pages of
> `getSignaturesForAddress`, ≈75,000 signatures — verified reachable by `before`-cursor
> paging **[M 2026-07-25]**. The census is affordable; the shortcut is not necessary.

**P2 — population prior.** All addresses receiving a value credit ≥ threshold in window
`W`. This estimates the class prior *any* pool inherits, and yields `ρ` and the
`eff_k(k)` curve. This is the generalizable claim and it is the one to lead with.

### 3.2 Frame construction for P2: sample slots, not consecutive blocks

`getBlock` with `transactionDetails: "accounts"` returns exactly
`{transaction:{accountKeys,signatures}, meta:{err,fee,preBalances,postBalances,
preTokenBalances,postTokenBalances,status}}` — precisely the projection the edge
extractor needs — at **2.73 MB/block vs 6.41 MB for `"full"`**, 1,056 transactions,
1.6 s fetch **[M 2026-07-25, slot ≈435,066,350]**.

**Procedure.** Draw `N_blocks` slots uniformly at random across `W` using the seeded
PRNG (§3.5), `getBlock` each, extract all value edges. 1,000 sampled blocks ≈ 1M
transactions ≈ 2.7 GB, ~1,000 calls. This is a genuine probability sample of chain time.

> **Failure mode.** A sample drawn from *consecutive* slots is not a sample of chain
> time. An n=1,181 sample spanning 9 consecutive slots covers ~3.2 seconds of chain
> time; the large n makes it look powerful, and binomial Wilson intervals computed over
> it look rigorous, but the observations are neither independent nor representative of
> anything but that instant. Sampling slots uniformly across the window costs the same
> number of calls and fixes it, so there is no efficiency argument for the shortcut.

### 3.3 Time stratification

- Window `W = [S − 19,440,000, S]` ≈ **90 days = 45 epochs**. Solana fixes an epoch at
  **432,000 slots** with a ~400 ms nominal slot **[V]**; Alpenglow had not reached
  mainnet as of mid-2026 and slot times are unchanged. Do **not** assume 400 ms —
  compute the realized mean from `blockTime` deltas over `W` and record it in the
  manifest.
- **45 epoch strata** (or 90 daily strata). Allocation proportional to stratum activity;
  record the allocation vector in the manifest.
- Canonical total order within a stratum: `(slot, transactionIndex, signature)`.
  `getSignaturesForAddress` returns a `transactionIndex` field **[M 2026-07-25]** (not
  in the RPC reference) which gives deterministic intra-slot ordering at no cost.

### 3.4 Sample size

`n = z²p(1−p)/w²`, finite-population correction `n/(1 + (n−1)/K)`:

| 95% half-width | n (p=0.5) | FPC K=2,000 | FPC K=10,000 |
|---|---|---|---|
| ±10 pp | 97 | 92 | 96 |
| ±5 pp | 385 | 323 | 370 |
| **±3 pp** | **1,068** | **697** | **965** |
| ±2 pp | 2,401 | 1,092 | 1,937 |
| ±1 pp | 9,604 | 1,656 | 4,900 |

**Baseline `n = 1,100`; stretch `n = 2,400`.**

For scale: a published 11/30 root-hit rate carries a Wilson 95% CI of
**[21.9 %, 54.5 %] — 32.6 percentage points wide**.

**Confidence intervals.**

- Effective-k has no closed-form CI. Use a **stratified block bootstrap resampling
  whole clusters** (funder-cluster or slot), `B = 10,000`, BCa intervals. Report the
  design effect.
- **CI width must be a function of data collected, never of loops run.** The failure
  looks like this: a published interval of `[+0.308, +0.325]` turns out to be exactly
  `2·1.96·√(p(1−p)/B)` for `B` the resampling count — so the interval narrows as the
  loop count rises and can be made arbitrarily tight without collecting one extra
  byte. It is easy to ship because a tighter interval reads as a better measurement.
  Write a test that fails if the reported width moves when only the resample count
  does.

**Entropy bias.** Plug-in `Ĥ` is downward-biased by ≈`(m−1)/(2n ln2)` bits. Emit
plug-in, Miller–Madow, and Chao–Shen (coverage-adjusted) side by side. Emit Good–Turing
coverage and Chao1 as saturation diagnostics; on Vector B they read 63.3 % coverage and
67 estimated classes from a sample of 30, which is how you say "this sample is nowhere
near saturating" in one number.

### 3.5 Seeding

```
seed = blockhash(S)          // public, fixed before we chose it — un-grindable
prng = ChaCha20(seed)
```

Record `S`, `blockhash(S)`, and the derivation rule in the manifest.

### 3.6 Never filter the frame on the outcome

> **Failure mode.** Discarding every candidate depositor with ≥1,000 signatures looks
> like a reasonable cost control — those are the expensive ones to walk — but it drops
> exactly the wallets with deep, traceable histories. Whatever remains is then
> disproportionately untraceable, and a headline like "63 % untraceable" is measuring
> the exclusion rule. **An exclusion criterion correlated with the outcome is not a
> filter, it is the finding.**

**Rule: nothing is excluded from the frame.** High-activity members are stratified and
reported, never dropped. If a member turns out to be a relayer or a program, that is a
finding, labelled as such, kept in the denominator.

Related: attribute deposits by **value flow** (the account whose lamport delta went
negative into the pool), not by `accountKeys[0]`. `accountKeys[0]` is the *fee payer*;
any relayer-paid deposit is otherwise attributed to the relayer. Emit the
fee-payer/value-source disagreement rate as a statistic.

---

## 4. Edge extraction and tracing

### 4.1 Edge extraction: balance deltas, not instruction parsing

Parsing `parsed.type == "transfer" && program == "system"` misses:

1. programs that move lamports by direct account mutation
   (`**acct.lamports.borrow_mut()`), which emit no system-transfer instruction at all;
2. `createAccount`, `createAccountWithSeed`, `transferWithSeed`, `withdrawNonceAccount`;
3. `closeAccount` lamport returns;
4. all SPL token flow.

**Verified [M 2026-07-25]:** with `encoding: "jsonParsed"`, `accountKeys` *does* include
lookup-table-loaded addresses — a probed v0 transaction had 11 `source:"transaction"` +
9 `source:"lookupTable"` = 20 keys — and `preBalances`/`postBalances` both have length
20, aligning with the **full resolved key list**. `meta.loadedAddresses` is absent under
`jsonParsed` (folded into `accountKeys`). The RPC reference text implies otherwise; the
empirical result is authoritative. **Pin this in a test against a committed fixture.**

```
for each account index i in accountKeys:
    Δ_i = postBalances[i] − preBalances[i] + (meta.fee if i == 0 else 0)
sources = { i : Δ_i < 0 }
sinks   = { i : Δ_i > 0 }
```

Token flow: diff `preTokenBalances` / `postTokenBalances` by `(accountIndex, mint)`.
Each entry carries `owner`, giving the owner wallet directly from `meta`.

Attribution within a transaction: single-source ⇒ unambiguous. Multi-source ⇒ record the
source *set* and set `ambiguous_attribution = true`. **Emit the ambiguity rate.** Use
instruction parsing only as corroboration, never as the primary extractor.

Shape note: under `transactionDetails: "accounts"` the path is
`transaction.accountKeys`; under `"full"` it is `transaction.message.accountKeys`.

### 4.2 The SPL blind spot — scope it explicitly

`getSignaturesForAddress` indexes on `accountKeys` only **[V]**. A plain SPL transfer
into an existing ATA does **not** name the recipient wallet. Verified on three real
mainnet USDC transfers **[M 2026-07-25]**: the destination token account was in
`accountKeys` in 3/3 cases; the destination's **owner wallet was in 0/3**. Helius's own
documentation states it: *"Does not include associated token accounts — use
getTransactionsForAddress for complete token history."*

**Consequence.** A wallet funded in USDC/USDT — the dominant CEX withdrawal path on
Solana — may have no signature history at its wallet address for the funding event. A
SOL-only tracer is structurally blind to it and the resulting "unresolved" is an
artifact, not a finding.

Two supported scopes. **The manifest MUST record which one produced the headline.**

- `scope = "sol"` — native SOL only. Cheaper; the blind spot is a stated limitation.
- `scope = "sol+spl"` — enumerate `getTokenAccountsByOwner` per address and page
  `getSignaturesForAddress` on each ATA. **2.5–3× the call count** (§8).

**Honest limitation to print in the output, either way:** a wallet that received tokens
into an ATA it later closed is invisible to `getTokenAccountsByOwner`. Bound the
residual by cross-checking against the block-sampled frame (§3.2) and report it.

### 4.3 The edge rule: the birth edge

**Primary rule.** For each address, follow the **oldest** value credit — the event that
created the account. Deterministic, single-valued, budget-independent, semantically
clean ("who created this wallet"), and one RPC call for most addresses.

`getSignaturesForAddress` returns **newest-first**, and the only cursors are `before`
(page backwards) and `until` (stop early); there is no forward cursor and `limit` caps
at 1,000 **[V]**. So for an address with fewer than 1,000 lifetime signatures, the
**last element of the first page** is the oldest transaction.

**The oldest transaction is not the birth edge.** It is where the search for the birth
edge starts. An address routinely appears as a passive participant in someone else's
transaction — an associated-token-account creation, a multisig configuration, a program
call that merely references it — *before* it ever receives value, and in that case its
oldest transaction carries a lamport delta of exactly zero for it.

So the walk continues **forward** from the oldest until a crediting transaction appears,
bounded by `BIRTH_SCAN_CAP`. Exhausting that budget without finding a credit is
`birth-scan-exhausted`, which is **budget-unresolved**, and it is a separate outcome
from `no-incoming-edge` for a reason that is the whole discipline of §6 in miniature:

| outcome | claim | about |
|---|---|---|
| `no-incoming-edge` | this address has no funding credit | the chain |
| `birth-scan-exhausted` | we stopped reading before finding one | us |

> **Failure mode, and this one was ours.** The first implementation here fetched *only*
> the oldest transaction and, if it was not a credit, reported `no-incoming-edge`. Two
> seeds checked by hand had 6,457 and 11,491 lifetime signatures, were plainly funded,
> and were reported as having no funding at all — because one transaction out of eleven
> thousand had a zero delta. The measured cost was a **six-fold** inflation of the
> evidence-unresolved bucket in the population where it bit hardest, and the reason it
> survived review is that the output was indistinguishable from a real finding.
>
> It is recorded here rather than quietly repaired because a document that lists other
> people's failure modes and omits its own is not a method, it is marketing.

> **Failure mode.** Scanning an address's most recent transactions to find its funder.
> `getSignaturesForAddress` returns newest-first, so taking the first handful is the
> path of least resistance — and funding is by definition among an address's *oldest*
> transactions. For any address with more history than the scan window, this reads the
> wrong end of the record, and it fails **silently**: the output is "unresolved", which
> is indistinguishable from a genuinely unresolvable wallet. The bias falls hardest on
> active wallets, which are the ones a provenance study most needs to resolve.

**Independent corroboration of the rule.** Dune Spellbook contains
`addresses_events_solana.first_funded_by`, described in its own `schema.yml` as *"Table
showing who first funded each Solana address in SOL"* — i.e. the canonical open
implementation of this question also resolves to the *first* funding event. It sources
`system_program_call_Transfer` and is therefore **native-SOL-only**, inheriting exactly
the blind spot in §4.2. See §7.4 for how we use it (cross-check, not collection path).

**Secondary rule, for sensitivity:** the **max-value incoming edge within `W`**. Run
both; emit the disagreement rate.

**Do not use a set-valued class key.** A key built from "the set of hubs reached", under
a per-address funder cap and node budget, makes two identical wallets receive different
keys purely from budget-exhaustion ordering.

### 4.4 Traversal

```
trace(addr, depth):
    if depth > DEPTH_MAX:            return Unresolved(DepthExceeded)
    if terminal(addr):               return Terminal(rule, id)      # §4.5
    edge = birth_edge(addr)                                          # §4.3
    if edge is None:                 return Unresolved(NoIncomingEdge)
    if edge.value < MIN_EDGE:        return Unresolved(BelowThreshold)
    return trace(edge.source, depth + 1)
```

Single-edge following means no branching factor and no node budget — both of which were
sources of nondeterminism in prior work. Emit the **depth distribution**, not just the
mean.

### 4.5 Terminal detection (`terminal(addr)`)

Evaluate in order. **Record which rule fired** — the class label carries it, so every
rule can be ablated in the sensitivity table (§5.5).

| rule | test | class label |
|---|---|---|
| **R1 program/PDA** | `getAccountInfo(addr).owner ≠ 11111111111111111111111111111111`, or `executable == true` | `program:<id>` |
| **R2 curated anchor** | hit in the Tier-2 anchor set (§5.4) | `entity:<name>` |
| **R3 structural cluster** | member of a fee-payer / sweep cluster of size ≥ 3 (§5.3) | `cluster:<canonical_id>` |
| **R4 volume hub** | lifetime signatures ≥ `T_HUB` (measured, bounded — §4.6) **and** account age ≥ 30 d | `busy-unlabelled:<addr>` |
| **R5 distributor** | has funded ≥ `F_FANOUT` distinct previously-unseen addresses | `distributor:<addr>` |
| — | none fired, budget exhausted | `unresolved(reason, depth, flags)` |

**R1 is definitional and cheap** — one `getAccountInfo` — and it is what separates
"funded by a Raydium vault" from "funded by a person". Skip it and those two collapse
into one class, which corrupts the distribution the headline is computed over.

**`busy-unlabelled` is not "an attributable origin". Never call it one.**

> **Failure mode.** Setting `HUB_THRESHOLD == SIG_LIMIT` — one constant for "how many
> signatures make this a hub" and "how many signatures will we fetch" — is a natural
> collapse, because both answer the same-sounding question. It makes "reaches an
> attributable origin" mean literally "**the address hit the RPC page cap**", which
> admits every DEX program, AMM vault, MEV bot and staking pool as an origin, while
> excluding a genuine CEX withdrawal address with 800 transactions. The two constants
> answer different questions and must be allowed to disagree.
>
> R4 here decouples the hub test from the page cap by measuring the count with bounded
> paging, and requires corroboration from R1/R2/R3/R5 before any class is called an
> entity.

### 4.6 Pagination handled honestly

For an address with `N` lifetime signatures, reaching the oldest costs `⌈N/1000⌉` calls.
Page up to `SIG_PAGE_CAP`. If the cap is hit, the address is by construction
high-activity and becomes a **terminal hub candidate** — classified by R1/R2/R3/R5, and
labelled `busy-unlabelled` only if nothing else fires. Set `flags.page_cap_hit = true`
and record the observed signature count as a **lower bound** (`sigs ≥ 10000`), never as
an exact value.

The page cap becomes a *classification signal with a named rule*. It never becomes a
silent "unresolved".

Measured page-cap incidence on a naive sample of SOL-credited destinations from one
mainnet block: **4/9 resolved in a single call, 5/9 page-capped** **[M 2026-07-25,
n = 9 — small; treat as an order-of-magnitude estimate]**. Depositor wallets are
typically fresher than raw transfer destinations, so measure and report the rate for the
actual population rather than assuming.

### 4.7 Parameters (v1)

| parameter | value | rationale |
|---|---|---|
| `SNAPSHOT_SLOT S` | last finalized slot of a named epoch | pins the ledger; its blockhash seeds the PRNG |
| `WINDOW` | 19,440,000 slots ≈ 90 days = 45 epochs | spans regimes; ≫ one CEX operations cycle |
| `DEPTH_MAX` | 6 | observed mean depth ≈1–2; headroom is cheap under single-edge following |
| `MIN_EDGE_SOL` | 1,000,000 lamports (0.001 SOL) | above the 890,880-lamport rent-exempt floor; excludes dust-spam |
| `MIN_EDGE_SPL` | ≈$1 at a **pinned, committed** price table | prices must be pinned or results drift |
| `SIG_PAGE_CAP` | 10 pages (10,000 signatures) | bounded; flagged, never silent |
| `T_HUB` | 5,000 lifetime sigs **and** age ≥ 30 d | decoupled from the page cap; sweep 1k/5k/20k in sensitivity |
| `F_FANOUT` | 50 distinct fresh fundees | distributor detection |
| `N_BLOCKS` | 1,000 | frame construction (§3.2) |
| `B_BOOTSTRAP` | 10,000 | CIs |
| `scope` | `"sol"` \| `"sol+spl"` | §4.2 — MUST appear in manifest |

---

## 5. Label tiers and multi-resolution reporting

A single-resolution number is the structural weakness of most provenance metrics:
report one figure and it is unclear whether it describes an adversary who sees raw
addresses or one who sees named entities, and those differ by orders of magnitude.
Reporting the full ladder is what makes the claim falsifiable, so this section is
specified in full.

### 5.1 Four resolutions, strictly nested

| L | resolution | derived from | what merges |
|---|---|---|---|
| **L0** | raw address | nothing | nothing. Finest ⇒ **lowest** eff-k ⇒ strongest adversary |
| **L1** | wallet-entity | ATA→owner (from `meta.*TokenBalances.owner`); co-signer clustering; R1 program/PDA detection | a wallet with its token accounts |
| **L2** | named entity | fee-payer clustering + sweep clustering + Tier-2 anchors | Binance hot wallets #3 and #7 → `Binance` |
| **L3** | entity category | hierarchy over L2 | CEX / bridge / DEX-AMM / lending / launchpad / MEV-bot / staking / unlabelled-hub / unresolved |

Enforce the hierarchy in the type system so `L0 ⊑ L1 ⊑ L2 ⊑ L3` holds by construction.
Then

```
eff_k(L0)  ≤  eff_k(L1)  ≤  eff_k(L2)  ≤  eff_k(L3)
```

is a theorem, not a hope, and the ladder is a genuine bracket. **Assert it at runtime**;
a violation means the hierarchy was broken and the run must fail.

**Headline at L2** — the finest two-sided resolution (§1.2). L0/L1 are published as the
strong-adversary bound, L3 as the weak.

*Not shipped.* The crate classifies each address once, by the R-rules of §5.3,
and reports one ρ. It does not re-label the same sample at four granularities, so
there is no L-rung bracket and no run has ever asserted the monotonicity above.
The published headline carries the resolved/unresolved bracket of §2.6 instead,
which brackets a different uncertainty: how much the members that failed to
resolve could move the number, not how much a coarser labeller would.

### 5.2 Atoms vs crowds — the two-stage pipeline

`2^{H(C)}` and `2^{H(X|C)}` are complements, and which is correct depends on whether
class members are mutually anonymizing:

| the class means | members are | correct metric |
|---|---|---|
| **crowd** — distinct users sharing a funder (all withdrew from one CEX hot wallet) | mutually anonymizing | `2^{H(X\|C)}` (residual) |
| **atom** — one principal operating many addresses | **not** mutually anonymizing | `2^{H(C)}` (the class *is* the secret) |

Pick wrong and the headline inverts. With `K = 100`: a partition of one class of 91 plus
9 singletons gives `2^{H(C)} = 1.65` and `2^{H(X|C)} = 60.6`; total deanonymization
(100 singletons) gives `2^{H(C)} = 100` and `2^{H(X|C)} = 1.0`.

**Both kinds occur in our data.** L1 clustering produces **atoms** — those addresses
genuinely are one principal. L2 classes are **crowds** — "funded by Binance" is
thousands of distinct people. Therefore:

1. **Collapse stage (atoms).** Apply L0→L1 clustering to reduce `K` raw member addresses
   to `K'` distinct principals. Emit `K'/K` as the **Sybil deflation factor**: advertised
   k is already overstated before provenance enters. All downstream metrics use `K'`.
2. **Partition stage (crowds).** Partition the `K'` principals by L2/L3 provenance class
   and compute §2.

State the assumption in one sentence in the output: *"we treat co-signing / ATA-linked
addresses as a single principal, and same-funder addresses as distinct principals; the
effect of each assumption is reported separately."*

### 5.3 Tier 0 and Tier 1 — derived, zero license risk

**Tier 0 — definitional, exact, from chain:**

- ATA → owner, read directly from `meta.pre/postTokenBalances[i].owner`.
- R1 program/PDA/executable detection via `getAccountInfo`.

**Tier 1 — structural, derived by our own committed code:**

- **Fee-payer clustering.** On Solana `accountKeys[0]` is both fee payer and a required
  signer. Exchanges pay withdrawal fees from a small set of fee-payer accounts
  regardless of which hot wallet sources the value. Clustering on fee payer **collapses
  an exchange's hot wallets into one entity for free, with no third-party label list.**
  This is the Solana-specific unlock, and it is the merge most easily missed: without
  it an exchange appears as a dozen unrelated classes, which inflates the measured
  class count and flatters the pool.
- **Co-signer clustering.** A transaction with signers `{A, B}` means one party controls
  both.
- **Sweep clustering.** If addresses `A_1 … A_n` each send their full balance to `H`,
  they are deposit addresses of `entity(H)` — the Solana analogue of common-input-
  ownership clustering.

All three are deterministic, auditable, reproducible, and MIT because they are our code
and our derived artifact.

### 5.4 Tier 2 — the shippable anchor set (~418 entries, all MIT-verified)

| source | license | content |
|---|---|---|
| `solana-foundation/explorer` → `public/verified-programs.json` | MIT | **288** verified programs `{address,name,repoUrl,verifiedAt}` |
| `helius-labs/xray` → `config.ts` | MIT | 106 address→name mappings |
| `ashpoolin/gelato.sh` | MIT | 24 Solana CEX addresses |
| `0xB10C/ofac-sanctioned-digital-currency-addresses` | MIT | `sanctioned_addresses_SOL.json` |

The specification's record shape, one per address — the anchor set that shipped
lives in `classify.rs` rather than in a file:

```json
{
  "address": "…",
  "entity": "Binance",
  "category": "cex",
  "source_url": "https://github.com/…",
  "source_license": "MIT",
  "evidence": "fee payer for withdrawals from 5tzFki… and 9WzDXw…",
  "verified_at_slot": 435071454,
  "confidence": "high"
}
```

The specification called for `verify-labels`, re-checking each anchor against the
chain (e.g. that a claimed exchange fee payer still pays for withdrawals from its
claimed hot wallets), because **an anchor list must be falsifiable, not
asserted**. *This did not ship.* The anchor set is in `classify.rs` and every
entry cites its source, but nothing re-derives it from the chain, so a stale
anchor would go unnoticed. It is the largest unclosed gap in the label ladder
and the reason Tier-2 results are reported beside Tier-1 rather than instead of
them.

**OFAC.** Use the **XML** feed, not the CSV: the CSV export omits them.
`https://sanctionslistservice.ofac.treas.gov/api/PublicationPreview/exports/SDN.XML`
(28.9 MB, publish date 2026-07-24, 19,254 records) contains 963 crypto addresses — XBT
524, TRX 195, ETH 96, USDT 93, LTC 14, XMR 11, **SOL 3** **[M 2026-07-25]**. Public
domain under 17 U.S.C. §105. Three addresses is not a labelling strategy; it is a
high-confidence anchor and nothing more.

### 5.5 Tier 3 — used for sensitivity, never shipped, never in the headline

Anything whose provenance or license we cannot stand behind. Specifically:

- **Dune Spellbook `cex_solana.addresses`** — Business Source License 1.1 (Licensor: Dune
  Analytics AS; Change Date 2027-03-03 → GPLv3+; "you may not use the Licensed Work for a
  Data or Analytics Platform") **[V]**. **Not MIT-compatible; do not vendor.** The Solana
  content was added after the MIT→BSL switch, so no MIT snapshot exists; after March 2027
  it is GPL, still incompatible with an MIT crate. It holds 166 rows and is noisy
  (contains a hex string, a Bitcoin bech32 address, uppercase-mangled entries). Note also
  that Spellbook's `labels_cex.sql` unions **only EVM chains — there is no Solana in
  Dune's `labels.cex`**, so "just use Dune's labels" does not survive contact regardless
  of licensing.
- **`apostleoffinance/Solana-Forensic-Analysis-Tool` → `solana_cex_labels.csv`** — MIT as
  published, 99,999 valid base58 addresses of which 99,986 are `deposit_wallet`, across 19
  exchanges. Deposit-address coverage at that scale is exactly what L2 wants and no free
  alternative has it. **But the provenance does not hold up:** the columns are Flipside's
  `solana.core.dim_labels` schema verbatim and the row count is exactly 100,000 + header,
  the signature of a `LIMIT 100000` export. A third party applied MIT to someone else's
  data. Flipside is defunct (see §8.3), so practical risk is low, but "the licensor is
  defunct" is not a provenance story that belongs under a headline number.
- **Commercial labels** (Arkham, Nansen, Solscan) — Solscan's ToS forbids redistribution;
  the others are proprietary. `solscanofficial/labels` is first-party with 1,169 addresses
  but `license: null` **[V]** — worth opening an issue requesting MIT.

**Tier-3 output is a single sensitivity line**, e.g. *"with a 100k-row third-party
deposit-address set of uncertain provenance, the L2 effective-k moves from X to Y"* —
with the set not redistributed. The *size* of that movement is the most informative
statement we can make about how much a free-label result understates a funded adversary.

### 5.6 The sensitivity table (specified, not produced)

No command emits this table. It is kept because it states what a complete report
of this method looks like, and because the gap between it and what
`docs/MEASUREMENT_LOG.md` actually publishes is the honest measure of how far
the implementation got. The ablation rows are the more valuable half: they say
how much of the headline rests on each rule, and nothing in the runs answers
that.

```
resolution        classes m   ρ        eff_k_shannon   eff_k_minent   worst   coverage
L0 raw address        …       …            …               …            …        …
L1 wallet-entity      …       …            …               …            …        …
L2 named entity  ←hl  …       …            …               …            …        …
L3 category           …       …            …               …            …        …

ablations (at L2):
  −R2 anchors         …       …            …               …            …        …
  −R3 clusters        …       …            …               …            …        …
  −R4 volume hubs     …       …            …               …            …        …
  T_HUB = 1k / 5k / 20k …
  Tier-3 labels on    …       …            …               …            …        …
  scope = sol / sol+spl …
```

Each row also carries the §2.6 bracket. The monotonicity assertion of §5.1 applies down
the first four rows.

---

## 6. Fail-closed failure accounting

**Principle: an infrastructure limit must never read as an absence of evidence.**

> **Failure mode.** `.unwrap_or(0)` on a signature count and `.unwrap_or_default()` on a
> signature list. Both are the idiomatic way to keep a traversal running past an error,
> and together they are silently catastrophic: a rate-limited call makes an address look
> like it has *zero* signatures, so it escapes hub detection and is admitted to the
> sample; and it makes a traced address yield no funders, so the walk terminates and the
> member is recorded **unresolved**.
>
> The result is that **an infrastructure failure is laundered into a data point**, and
> in the direction that inflates the unresolved bucket. Given the 429 behaviour measured
> in §8.1, aggressive inter-call pacing against a metered endpoint produces exactly the
> large unresolved bucket that systematic rate limiting would manufacture — which is
> indistinguishable, in the output, from a genuinely hard-to-trace population.
>
> Counting the failures and printing a warning is not sufficient. If the metric function
> never consults the count before reporting, the headline is still unconditioned on
> whether the run worked. Here the count gates the headline (§6) rather than annotating
> it.

### 6.1 Startup preconditions (hard)

```
assert getFirstAvailableBlock() == 0
assert getTransaction(CANARY_2021_SIG) != null
record endpoint, commitment, and both results in manifest.json
```

Refuse to run if either fails.

Rationale **[M 2026-07-25]**: the Solana Labs public endpoint **is** archival —
`getFirstAvailableBlock = 0`, `getBlock(50,000,000)` OK at blockTime 2020-11-19, and
`getSignaturesForAddress(addr, before=<Oct-2021 sig>)` returns *older* 2021 signatures,
so the signature index is archival too. But third-party "public" endpoints are not:
PublicNode reports `getFirstAvailableBlock = 434,525,787` against current slot
435,068,692 — a **2.51-day window** — and `getTransaction` on a 2021 signature returns
**`{"result": null}`, not an error**. A collector pointed at it records "transaction not
found" and produces a silently truncated graph that looks like a finding.

### 6.2 Failure taxonomy

Every terminal state is one of these and is counted separately. There is no "other".

| state | meaning | counts toward |
|---|---|---|
| `Terminal(rule, id)` | classified | resolved |
| `Unresolved(NoIncomingEdge)` | genuine: address has no qualifying incoming value edge | unresolved — **evidence** |
| `Unresolved(BelowThreshold)` | incoming edge below `MIN_EDGE` | unresolved — evidence |
| `Unresolved(DepthExceeded)` | hit `DEPTH_MAX` | unresolved — **budget** |
| `Unresolved(PageCapHit)` | hit `SIG_PAGE_CAP` without terminating | unresolved — budget |
| `Unresolved(RpcFailure)` | any RPC error, 429 exhaustion, or timeout | **failure — not evidence** |
| `Unresolved(SplBlindSpot)` | SOL scope, address has token accounts with inflow | unresolved — scope |

### 6.3 Reporting rules

1. `Unresolved(RpcFailure)` members are reported on a **separate census line** and are
   excluded from the class distribution entirely. They are neither merged nor split.
2. **If the RPC failure rate exceeds 1 %, the run refuses to print a headline.** It
   prints the census and exits non-zero.
3. Budget-unresolved members (`DepthExceeded`, `PageCapHit`) enter the §2.6 bracket, not
   a class.
4. The manifest records total calls, failures by type, retries, and the observed 429
   rate.
5. Emit `reliable: failures == 0` as a machine-readable field, and never print a
   headline number in the same object as `reliable: false`.

---

## 7. Determinism, artifacts, and replay

### 7.1 Snapshot semantics

Everything is filtered client-side by `slot ≤ S`. `minContextSlot` is a freshness
guarantee, not a filter **[V]**, so it cannot do this. Finalized history below `S` is
immutable, so the filter is deterministic and a replay run at any later date reproduces
the same set.

### 7.2 Record / replay cache

- `MIRROR_RPC=record` — hits the network, appends to the store.
- `MIRROR_RPC=replay` — runs **entirely offline** from the committed store. This is the
  default in CI.

Content-addressed: `data/raw/<sha256>.json`, with an index mapping
`(method, canonical_params) → sha256`. Canonical params = JSON with sorted keys, no
whitespace.

*Specified, not shipped.* `rpc.rs` implements the `slot ≤ S` filter, the paging
and the failure taxonomy below; the on-disk cache is not built, because no run
here was large enough to need one. The runs are reproducible from their
committed sample files instead — see §7.3.

### 7.3 On-disk format

A `jsonParsed` `getTransaction` is 10–60 KB; 50k of them is ~1.5 GB, which does not
belong in git. **Commit the projection plus the hash of the original.**

`data/tx/<sig>.json` (projection, ~1–3 KB):

```json
{
  "sig": "…", "slot": 435066575, "tx_index": 530, "block_time": 1784957226,
  "err": null, "fee": 5190,
  "account_keys": [{"pubkey":"…","signer":true,"writable":true,"source":"transaction"}],
  "pre_balances": [], "post_balances": [],
  "pre_token_balances":  [{"account_index":3,"mint":"…","owner":"…","amount":"…"}],
  "post_token_balances": [],
  "raw_sha256": "…"
}
```

**What shipped.** One file per run rather than the several this section
originally specified, because the runs are small enough that splitting them buys
nothing and costs a reader a join. `mirror collect` writes a single JSON
document with three top-level keys:

| key | contents |
|---|---|
| `manifest` | §7.5 — endpoint, archival probe, collection time, every threshold, the RPC call count, and the exclusion and ambiguity rates |
| `chains` | one traced funding chain per sampled address: the path, the edges, the terminal class, the rule that fired, and the failure flags |
| `facts` | the sampled address set, in selection order |

The runs published here are in `data/sample-*.json` — around 200 KB each at
n ≈ 80–100 traced chains, which is small enough to commit whole. `mirror
analyze` reads one of these and computes every metric in §2 from it, so a reader
can rerun the analysis without touching an RPC endpoint, and `mirror compare`
takes two.

The record/replay design in §7.2 exists so that a bigger collection could ship
its raw responses separately with checksums recorded in-repo. No run here is
large enough to need that.

### 7.4 `first_funded_by` as cross-check, not collection path

Dune's `addresses_events_solana.first_funded_by` precomputes our birth edge for every
Solana address. It does **not** become the collection path, for three reasons:

1. It sources `system_program_call_Transfer` — **native SOL only** — so it inherits the
   §4.2 blind spot and cannot cover the USDC/USDT funding path either.
2. Spellbook is BSL-1.1 (§5.5). Querying the hosted table is use, not redistribution, and
   is fine; vendoring the model is not.
3. A headline derived from a BSL-licensed SQL model we cannot ship or audit trades away
   the one thing that distinguishes this work.

**Correct use:** derive edges ourselves from RPC, then query `first_funded_by` for the
same sampled addresses and **publish the agreement rate as an external validation
statistic**. Two independently-implemented derivations of the same edge agreeing at X %
is stronger evidence than either alone, and each disagreement is individually
inspectable — it is either our bug or theirs. `solana.account_activity` (one row per
account per transaction, with `balance_change` and `token_balance_owner`) serves the same
cross-check on the SPL side, where `first_funded_by` cannot reach.

Dune free tier: 2,500 credits/month, 20 credits/MB export (~125 MB/month) **[V]** — enough
for a subsample cross-check, not for a full export. Join gotcha: `labels.*` stores
`address` as VARBINARY while `solana.*` uses base58 VARCHAR; use `from_base58()`.

**Do not use BigQuery as the substrate.** `bigquery-public-data.crypto_solana_mainnet_us`
has sufficient schema but: documented gaps of 13,602 missing blocks plus 18,879 blocks
with missing or duplicate transactions; `blockchain-etl/solana-etl` last pushed
2024-09-27; recurring silent stalls (froze 2025-03-31; a 6-day lag reported 2025-11-25
with no vendor response); and a documented case of a single Solana query billing $5,000.
If used at all, partition-filter everything and set `maximum_bytes_billed`. Google's
maintained `goog_blockchain_*` datasets cover nine chains and do not include Solana.

### 7.5 Manifest

```json
{
  "tool": "mirror-provenance", "version": "…", "git_commit": "…",
  "snapshot_slot": 0, "snapshot_blockhash": "…", "snapshot_block_time": 0,
  "window_slots": [0, 0], "realized_mean_slot_ms": 0.0,
  "population": "P1_pool_census | P2_population_prior",
  "pool_program": "…",
  "strata": "epoch", "strata_allocation": [],
  "seed_rule": "blockhash(snapshot_slot)", "seed": "…",
  "scope": "sol | sol+spl",
  "params": { "DEPTH_MAX": 6, "MIN_EDGE_SOL": 1000000, "SIG_PAGE_CAP": 10,
              "T_HUB": 5000, "F_FANOUT": 50, "N_BLOCKS": 1000, "B_BOOTSTRAP": 10000 },
  "label_resolution_headline": "L2",
  "endpoint": "…", "commitment": "finalized",
  "precondition_first_available_block": 0,
  "precondition_canary_ok": true,
  "rpc_calls": 0, "rpc_failures": {"http_429": 0, "timeout": 0, "jsonrpc_error": 0},
  "reliable": true,
  "index_sha256": "…"
}
```

What shipped instead is `mirror analyze <run.json>`: it recomputes every published
number for a run from the committed file and touches no network, so any figure in
`docs/MEASUREMENT_LOG.md` can be re-derived from the artifact beside it. It is not
wired into CI, because the runs are data rather than code — a change to
`metrics.rs` that moved a published number would be caught by the §2.7 reference
vectors, which are.

---

## 8. RPC feasibility

### 8.1 Public endpoint — documented vs measured

Documented **[V]**: 100 req/10 s per IP, 40 req/10 s per IP for a single method, 40
concurrent connections, 100 MB/30 s, and *"not intended for production applications"*.

Measured against `getSignaturesForAddress` **[M 2026-07-25]**, two independent probes:

| pacing | result |
|---|---|
| concurrency 8, unpaced | **24/24 HTTP 429** |
| concurrency 4, unpaced | **24/24 HTTP 429** |
| concurrency 1, ~0.45 s gap (2.2 req/s offered) | 9 ok / **15 × 429** (62.5 % failure) |
| 1.0 s gap | 11 ok / 4 × 429 (27 % failure) |
| **2.0 s gap** | **15/15 ok** → 0.41 req/s clean |
| 3.0 s gap | 15/15 ok → 0.28 req/s |
| well-behaved, honoring `Retry-After`, 180 s | 99 ok / 2 × 429 → **0.55 req/s** |

**Sustainable rate: 0.28–0.55 req/s ≈ 1,000–2,000 req/hour, per IP** — roughly 7–10×
below the documented figure. Concurrency does not help; the cap is per-IP.

Report the measured range with the pacing method stated. Do not cite the documented
40 req/10 s, which neither probe could reproduce.

Other findings **[M 2026-07-25]**: `getProgramAccounts` returns HTTP 403 (we do not use
it). The 100 MB/30 s data cap is not binding — at 0.55 req/s the request cap binds ~30×
earlier.

### 8.2 Provider table

Call budget: `scope="sol"` ≈ 17k calls at n=1,100 / 30k at n=2,000;
`scope="sol+spl"` ≈ 45k at n=1,100 / 75–90k at n=2,000 (§4.2).

| provider | free allowance | effective rate | 45k calls | 90k calls |
|---|---|---|---|---|
| Solana Labs public | unmetered | 0.28–0.55 req/s **[M]** | ~23 h | ~45 h |
| **Helius Free** | 1M credits/mo, **1 credit/call**, archival genesis→present **[V]** | 10 rps | **1.3 h** (4.5 % quota) | 2.5 h (9 %) |
| **Alchemy Free** | 30M CU/mo, **40 CU/call** ⇒ 750k calls, full archive **[V]** | 500 CU/s ⇒ 12.5 calls/s | **1.0 h** (6 % quota) | 2.0 h (12 %) |
| QuickNode | 10M credits **one-month trial only**, 30 credits/call ⇒ 333k calls **[V]** | 15 rps | 0.8 h (14 %) | 1.7 h (27 %) |
| dRPC / Ankr keyless | **Solana blocked on free** **[M]** | — | — | — |
| Chainstack / GetBlock | archive **excluded** from free **[V]** | — | — | — |
| Blockdaemon (5 rps) / Shyft (1 rps) | — | unusable | — | — |
| Triton One | **no free tier**; $125 non-refundable minimum **[V]** | — | ~$0.90 at $10/M | — |

**Recommendation: Alchemy Free or Helius Free for the published run** — 1–2 hours, under
12 % of a monthly allowance, no payment. Retain the public endpoint as the zero-key
reproducibility fallback and the replay cache as the zero-network one.

### 8.3 Do not build on

- **Flipside** — defunct. `flipsidecrypto.xyz`, `.com` and `docs.` all 301 to
  `edisyl.com`; `api-v2.flipsidecrypto.xyz` fails to connect; `solana.core.dim_labels` no
  longer exists; `FlipsideCrypto/solana-models` is 404. Blockchain business sold May 2026.
- **PublicNode for history** — 2.51-day window, silent nulls (§6.1).
- **BigQuery without `maximum_bytes_billed`** (§7.4).
- **SolanaFM** — `api.solana.fm` returns 502 on every endpoint.

---

## 9. Honest-claims analysis

Our protocol intends to *reduce* this attack's effect. This section states what each
candidate mechanism actually closes, what it leaves open, and the experiment that would
honestly demonstrate it.

### 9.0 The anti-pattern, stated so it cannot be repeated

The tautological metric is the defining failure of this genre, and it is worth writing
out because it does not look like cheating while you are writing it. The shape is a
harness that branches on the scenario label *inside* the metric:

```rust
if scenario == Scenario::MirrorPool {
    return vec![0usize; k];      // "Provenance broken: one indistinguishable class."
}
```

Everything downstream is then an identity: the posterior is `vec![1.0; k]` and
`2^{H(Uniform(k))} = k`. The favourable column comes out as exactly 16.00 / 32.00 /
64.00 against a ragged baseline, and a unit test pinning that to 1e-6 reads as a
regression guard while actually pinning the tautology in place. The damning detail is
that the data structure typically *contains* the k distinct funding roots — the code
branches on the label and discards the field it should have read.

It is worth being precise about why this happens, because "they were dishonest" is the
least useful explanation. A harness is built scenario by scenario; the favourable
scenario is stubbed first to get the plumbing running; the stub returns the answer the
author expects; and nothing downstream ever fails, because a tautology cannot fail.

**Three tells a reader can check in thirty seconds**, and which no honest report produces:

1. a literal `if scenario == X { return <constant> }` inside a metric;
2. every cell in the favourable column being an exact integer while the unfavourable
   column is ragged;
3. a unit test asserting the favourable number equals nominal.

**The structural rule that prevents it.** The adversary MUST be a pure function of the
public transcript and MUST NOT receive the scenario label:

```rust
fn posterior(transcript: &[PublicRecord], target: ActionId) -> Vec<f64>;
```

No `Scenario` type may appear anywhere in the metric's call graph — enforce with a module
boundary. Every channel stays **on** and is **measured** to carry ≈0 bits, rather than
being gated off by a boolean. That converts an assumption into a testable property and
catches the case where the batch is not actually uniform. Ship a **negative control**:
inject a partial leak and assert the metric drops below nominal.

### 9.1 (a) Provenance-homogeneous cohort formation

*Only settle a round whose members share a provenance class.*

**Sound: yes.** It is the only one of the three that attacks the **membership-side**
channel. If every member of a settled round shares class `c`, then `μ(a) = c` is constant
across members, `I(X; μ) = 0` within the round, and `eff_k = k_round` exactly.

**Tautology risk: HIGH.** Measuring effective-k of homogeneous cohorts *with the labeller
that formed them* is `k_round` by definition. That is §9.0's defect written more
elegantly.

**Leaves open:**

1. **The round becomes the label.** Round `r` is now "the Binance round". No new
   intra-round leakage, but a user acting in two rounds is correlated across time by
   class. The intra-round leak is converted into an inter-round linkage.
2. **Minority members can never settle.** Under a heavy-tailed class prior most classes
   are singletons — the very members the mechanism was meant to protect cannot form a
   cohort at all.
3. **The guarantee is relative to a named labeller, not absolute.** If the adversary's
   `λ'` is strictly finer than our `λ_form`, the cohort is not homogeneous and the
   guarantee evaporates. Against a funded adversary this is a strong and probably false
   assumption.

**Honest experiment.** Using the measured mainnet class distribution, run formation policy
`π(k_min, W_max)` and report:

- (i) effective-k under `λ_form` — stating **up front** that this is `k_round` by
  construction and is not a finding;
- (ii) **the actual deliverable:** effective-k under a strictly finer, **held-out**
  labeller `λ_adv` never used for formation (adversarial-refinement stress test). This
  can be `< k_round` and is the real number;
- (iii) the **cost curve:** fraction of deposits settled, median and p95 wait by class,
  realized `k_round` distribution vs. the unconstrained pool.

The headline is a trade curve, not a point. Template — every letter is a placeholder to
be filled from our own measurement, never from anyone else's published figure:
*"at `k_min=8`, `W=24 h`, we settle X % of deposits at median k = Y with
adversarial-refinement effective-k of Z, versus an unconstrained pool at nominal k = A /
effective-k B."*

### 9.2 (b) Pool-executed actions

*The action is executed by the pool PDA, so the action's on-chain funding trace is the
pool, identical for all members.*

**Sound: yes — the strongest of the three**, but it closes a *different* channel from the
one being measured. Fee payer, signer, and the action's immediate funding source all
become the pool PDA. `μ(a)` is constant across members, so `I(X; μ(a)) = 0` and the
backward trace of the action is dead.

**Tautology risk: MEDIUM.** "All actions share the pool PDA as funder" *is*
near-definitional. It is not a tautology if and only if all three of these hold:

- the tracer is our general-purpose binary run **unmodified**, same parameters;
- there is a **negative control** — the same N members performing the same action
  *without* the pool, traced by the same binary, showing high recovery;
- the report is a **leakage decomposition across all channels**, not the single zero.

A number that is zero by construction is only interesting standing next to the numbers
that are not.

**Leaves open:**

1. **The forward trace.** The action has a beneficiary `D`; `D`'s subsequent behaviour and
   consolidation remain observable. (b) closes the backward trace and opens nothing new,
   but does nothing about the fate of the proceeds.
2. **Amount and timing.** If a member deposited 7.31 SOL and the pool later sends 7.31
   SOL, the linkage is trivial. **Once (b) is in place, amount/timing is the dominant
   channel** — the Tornado lesson. Fixed denominations, batching and delay are required
   and the residual must be measured, not assumed.
3. **Account-key co-occurrence.** If the beneficiary ATA is member-owned, the member is
   literally in `accountKeys`. Check explicitly.
4. **Who pays the fee.** If the member submits the transaction even though the PDA
   executes the action, `accountKeys[0]` *is* the member — total deanonymization. This is
   the number-one practical failure mode on Solana, and it is how a sampler harvests
   depositors from a pool that looks otherwise sound: the fee payer is in the clear on
   every transaction, so no proof system anywhere in the design helps. A relayer or
   pool-paid fees is mandatory, and the design must make member-paid submission
   **impossible**, not merely discouraged.

**Honest experiment.** Deploy, execute N actions from N distinct members, run the
unmodified tracer, and report per-channel leakage in bits with `provenance = 0.00` sitting
beside non-zero amount/timing/fee-payer figures, plus the no-pool control.

### 9.3 (c) Pool-as-funder

*Wallets funded by the pool all share one provenance class.*

**Sound: partially, with a sharp ceiling.** `λ(D) = "mirror-pool"` for every pool-funded
wallet, so the provenance partition over pool-funded wallets is a single class. That is
real, and it is the primitive any decoy-based design needs, because the
common-funder graph is what collapses a decoy set: decoys funded from one wallet
share a provenance class with each other and not with the member.

**But there is an exact bound, and it should be stated as a theorem:**

> Let `R` be funded by a single pool payout `π`, and let `A_π` be the set of members who
> could have caused `π`. `R`'s provenance is a deterministic function of `π`, so by the
> **data-processing inequality** the adversary's uncertainty about "who ultimately funded
> `R`" cannot exceed their uncertainty about "who caused `π`". Therefore
>
> **`eff_k(R's provenance) ≤ eff_k(the pool's membership set)`.**

**Pool-as-funder transfers the pool's anonymity downstream; it cannot manufacture
anonymity.** That is a genuinely useful service and an honest deflation of the pitch.

**Leaves open:**

1. **(c) is derived from (b), not independent.** The class is "everyone the pool ever
   funded" only if payouts are mutually indistinguishable. Distinct amounts and slots
   sub-partition it; without fixed denominations, batching and delay, the class collapses
   to one member per payout.
2. **Taint concentration.** One shared anonymity class is also one shared *taint* class.
   Pool-funded wallets inherit "funded by a mixer", which many exchanges flag. An
   adversary can deliberately deposit and withdraw to make the class conspicuous. This
   trades deanonymization risk for compliance risk and must be stated.
3. **Membership becomes maximally visible.** "Funded by the privacy pool" is itself highly
   informative — it says the recipient wanted privacy. (c) maximizes anonymity *within*
   the pool population while maximizing the visibility of *belonging to* it. Against the
   question "is this user privacy-seeking?" it increases leakage to 1 bit while decreasing
   it on "which user". It is stated here because a method that only reports the
   channel it improves is not a method.

**Tautology risk: MEDIUM-HIGH** if measured as "all pool-funded wallets have class =
pool" (definitional). Not a tautology if measured as:

- **the ceiling test** — does realized class size equal the pool's effective-k, or does it
  collapse toward 1 under amount/timing sub-partitioning?
- **an A/B against a decoy-based design** — run the tracer over the same decoy set
  *before* (funded from one wallet) and *after* (pool-funded), and report both
  effective-k values from the same binary. Hard to fake and easy to check, because
  the only thing that changed between the two numbers is the funding path.

---

## 10. Implementation

### 10.1 What a reader should check first

`metrics.rs` is §2 as pure functions over class-size multisets — the §2.7 reference
vectors, the ordering invariant and the negative control, with no RPC, no scenario
parameter and no I/O. It is the file to read first, because every number this
method reports passes through it and nothing in it depends on which chain the
data came from.

The claim that matters is not a smaller number than anyone else's. It is that a
reader can tell **which direction the error goes**, which is why the unresolved
and failure census (§6) is reported beside every estimate rather than after it.

### 10.2 Prior art, and where this sits in it

The funding-provenance channel is **not novel**. Wang et al., *On How Zero-Knowledge Proof
Blockchain Mixers Improve, and Worsen User Privacy*, WWW 2023, Heuristic H4 "Intermediary
Deposit Address" is this heuristic, published for Tornado Cash:

> "given two addresses `d⁽¹⁾` and `d⁽²⁾`, if all `d⁽¹⁾`'s coins are transferred from
> `d⁽²⁾` and `d⁽²⁾` is a user account, then `Link(d⁽¹⁾, d⁽²⁾) = 1`."

Measured effect on TC 0.1 ETH (|OAS| = 11,941): H4 alone −4.20 %; all five heuristics
combined −30.68 %. Their metric is Bayes vulnerability `Adv = 1/|SAS|` — identical to our
min-entropy rung.

Three things here are new, and each is stated so that a reader can check it rather than
take it:

1. **This is the first such measurement on Solana.** DBLP's full index returns six Solana
   blockchain papers — phishing, rug detection, transaction failure, Jito MEV, the NFT
   ecosystem, SolRPDS — and **none** on privacy, anonymity, deanonymization, address
   clustering, mixers or shielded pools. arXiv full-text agrees. Of the protocols
   themselves: Elusiv sunset on 2024-02-29 (its team moved to Arcium and general MPC) and
   was never measured; Light Protocol pivoted to ZK Compression; Privacy Cash, launched
   2025-08-27 and the most-used Solana ZK mixer, has never been measured. The search is
   reproducible and a counter-example would settle it.
2. **The label ladder is entity-level, with its sensitivity reported.** Prior work,
   Wang et al. included, fixes a single resolution and reports the number it produces.
3. **`ρ = 2^{−H(C)}` is a k-independent headline.** Prior work reports a percentage
   reduction for one pool at one size, which cannot be compared across pools or
   extrapolated to a larger one.

For scale: SPL Token-2022 confidential transfers hide *amounts and balances only* — sender
and receiver addresses stay public — so the advertised anonymity set is exactly 1 by
construction. Solana's flagship privacy primitive has no anonymity set, and the one
protocol class where an advertised-versus-effective gap could exist had not been studied.

**Comparanda for our result:** Tutela (arXiv:2201.06811) −37 % ± 15 %; Wang et al. −27.34 %
(ETH) / −46.02 % (BSC); Béres et al. (IEEE DAPPS 2021) anonymity set 400 → ~12 under a
one-day timing assumption; Kappos et al. (USENIX Security 2018, Zcash) −69.1 % **of value,
not of members**; Möser et al. (PoPETs 2018, Monero) ring size 11 → effective 1.16–1.80.

One paper is deliberately absent from those comparanda. arXiv:2510.09433 (Cristodaro,
Kraner & Tessone, Tornado Cash cross-chain clustering) was **withdrawn** at v3 on
2025-11-18 — *"This paper has been withdrawn by the author due to mistakes in the
references"*. Its numbers still surface in search results and are easy to pick up
second-hand, so the withdrawal is recorded here rather than the paper being silently
left uncited.

**Terminology.** "Provenance class" is not standard. Define it once against the
established vocabulary: *"we partition the anonymity set into* **provenance classes**,
*the equivalence classes induced by the common-funder relation; these correspond to*
clusters *in the address-clustering literature (Meiklejohn et al., IMC 2013; Victor, FC
2020) and to the blocks of a* partition gain function *in the QIF sense (Alvim et al.,
CSF 2012, §III-B-2)."*

Attribution note: common-input-ownership is **not** Meiklejohn's — they explicitly
disclaim it. The chain is Nakamoto §10 (observation) → Reid & Harrigan 2011 (graph
contraction) → Androulaki et al., FC 2013 (change addresses) → Meiklejohn et al., IMC 2013
(refined change heuristic, active re-identification, **peel chain** — that coinage is
theirs). For the account model the right citation is **Victor, FC 2020, deposit-address
reuse**, the direct ancestor of what we are doing.

### 10.3 Structural template

Huseynov, Shahzaib, Seres & Tapolcai, *A Tattered Cloak of Invisibility: Measuring
Anonymity Loss in Railgun on Ethereum*, arXiv:2606.25926 (June 2026) is the closest
existing work in form: it uses "nominal vs effective", cites both PET 2002 papers, defines
an optimistic upper bound `log2|D(w)|` beside the measured value, and reports a **3.42-bit
median anonymity loss** — a distribution, in bits, not a scalar. Imitate its structure.
