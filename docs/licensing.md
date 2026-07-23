# Licensing decision

Multichain is licensed under Apache License 2.0. Copyright is held by Rafal
Sikora and the public maintainer is RSI Tech.

## Why Apache-2.0 fits

This repository is infrastructure software intended to be studied, embedded,
modified, and operated commercially or privately. Apache-2.0 is a permissive,
OSI-approved license that provides explicit copyright and patent grants,
includes patent-litigation termination, and defines how attribution notices
are preserved in redistributions.

Those patent terms are the main reason to prefer it over MIT for a protocol and
data-infrastructure codebase. The `LICENSE` and `NOTICE` files are included in
source and binary distributions. Third-party dependency notices are generated
for every binary release.

## Alternatives considered

| License | Suitable when | Trade-off for Multichain |
| --- | --- | --- |
| MIT | Maximum simplicity and permissive reuse are the priority | Very short and familiar, but it has no express patent grant |
| MIT OR Apache-2.0 | Rust crate adoption across projects with either license is the priority | Flexible for downstream crates, but recipients may choose MIT and bypass Apache's patent terms |
| MPL-2.0 | Modifications to covered source files should remain open | File-level copyleft is a reasonable middle ground, with more compliance work for integrators |
| GPL-3.0 | Distributed derivative programs should remain under strong copyleft | Stronger reciprocity can reduce adoption in proprietary infrastructure stacks; ordinary server operation does not trigger source distribution |
| AGPL-3.0 | Modified network services must offer corresponding source to their users | Best defense against closed hosted forks, but the strongest adoption and legal-review friction |
| Source-available licenses | Commercial hosting or competitive use needs restrictions | Not an OSI open-source release unless and until the code converts to an open-source license |

Apache-2.0 is the recommended default for the current goal: an open,
commercially usable RSI Tech infrastructure project with clear patent terms.
If protecting hosted-service improvements becomes more important than broad
adoption, AGPL-3.0 would be the clearest future policy change and should be made
only after legal review and contributor-rights analysis.

## References

- [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0)
- [Apache guidance for applying version 2.0](https://www.apache.org/legal/apply-license)
- [OSI-approved licenses](https://opensource.org/licenses)
- [Mozilla Public License 2.0 FAQ](https://www.mozilla.org/MPL/2.0/FAQ/)
- [GNU explanation of AGPL network-source requirements](https://www.gnu.org/licenses/why-affero-gpl.html)

This document explains the project's engineering choice and is not legal
advice.
