# Archived blur experiments

Meaningful failed or inconclusive approaches are preserved on local experimental branches
before leaving the active implementation. These are source archives, not qualified releases.
The notes on each branch describe the idea, provenance, observed limitation and possible
reuse. Check `docs/blur-experiment.md` on the selected branch before using its code.

Earlier timings are specific to the tested Apple M1 Pro, resolution/AA and host conditions.
Several precede completion-paced headless profiling, so queue growth and memory pressure
can affect their interpretation. They are not universal rankings of the approaches.

| Branch | Preserved state | Potential reuse |
| --- | --- | --- |
| `codex/experiment/blur-quarter-native` | Complete recorded tracked/untracked source snapshot | Mixed-resolution camera filtering and independent object vectors |
| `codex/experiment/blur-cooperative-reduce` | Complete source/history; GPU serial comparator passes, Metal cost rejected | Shared-memory deterministic motion reduction on other GPU workloads |
| `codex/experiment/blur-viewmodel-reduction-region` | Complete wrap-up source; weapon-only Advanced and opt-in Ultra | Conservative bounds that skip empty reduction inputs; measured M1 direct drawing |
| `codex/experiment/blur-direct-drawing` | Complete source snapshot; opt-in profiling mode, native budget fails | Avoiding indirect phase preparation for smaller visible workloads |
| `codex/experiment/blur-main-depth` | Complete source snapshot; world/weapon probes pass, native cost pending | Sampling post-draw depth without a duplicated prepass export |
| `codex/experiment/blur-direct-colour-sources` | Complete source snapshot; world probes pass, native budget fails | Reading resolved post-process inputs without foreground copies |
| `codex/experiment/blur-tile-skip` | Complete recorded tracked/untracked source snapshot | Conservative empty-space traversal for sparse motion |
| `codex/experiment/blur-quarter-gather` | Exact shader snapshot; CPU integration incomplete | Lower-resolution filtering and alternate quality/bandwidth tradeoffs |
| `codex/experiment/blur-motion-occupancy` | Exact shader snapshot; CPU integration incomplete | Conservative motion masks and sparse sample consumers |
| `codex/experiment/blur-cache-sparse` | Exact shader snapshot; CPU resolve/binding harness incomplete | Sharing resolved motion/depth across repeated gathers |
| `codex/experiment/blur-cache-dense` | Reconstructed dense shader; CPU harness incomplete | High-MSAA or bandwidth/cache tradeoffs on other workloads |
| `codex/experiment/blur-single-sample-camera` | Recovered/reconstructed camera module; unwired | Single-sample auxiliary data while visible rendering retains MSAA |
| `codex/experiment/blur-encoder-timing` | Recorded timing patch reconstructed on a complete source snapshot | Backends with usable encoder-boundary GPU timestamps |

Shader-fragment branches add an unused `experiment_*.wgsl` source; switching to one does
not activate the historical experiment. The camera fragment is likewise unwired. These
labels prevent a partial recovery from being mistaken for a runnable historical checkout.
Archive checkouts have not been rebuilt or requalified; their original evidence and the
integration requirements are recorded in their branch notes.

The branch references retain the source objects independently of temporary stashes or
build packages. Generated executables, shader binaries, installed content and raw reports
remain ignored. Simple harness mistakes and no-op diagnostic ablations stay in the failure
notes rather than receiving separate implementation branches. Future meaningful variants
should be archived before replacement, with their context and uncertainty preserved.

Revalidation uses the owned GPU image probes and repeated, completion-paced, resource-aware
native-resolution benchmarks. Port relevant code into the active implementation only after
its behavior and costs have been checked for the new workload/platform.
