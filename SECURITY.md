# Security policy

## Supported version

Multichain is currently a developer preview. Security fixes are applied to the
latest commit on `main`; no older release line receives guaranteed fixes.

## Confidential reporting

Do not open a public issue for a suspected vulnerability.

Email [info@rsitech.ai](mailto:info@rsitech.ai) with:

- the affected commit or release;
- the component and configuration involved;
- reproduction steps or a minimal proof of concept;
- expected and observed impact; and
- any disclosure deadline already under consideration.

Reports are reviewed by RSI Tech, but this preview does not yet carry a response
time SLA. Do not include live credentials, private keys, wallet seed material,
personal data, or third-party production data unless a secure transfer method
has first been agreed.

## Scope and safety

Good-faith testing must use infrastructure and accounts you own or have
permission to test. Do not disrupt public blockchain networks, third-party
nodes, provider endpoints, or other users. This repository does not authorize
trading, custody, transaction signing, or access to external systems.

The local validation suite binds disposable services to loopback interfaces.
Production deployments require separate hardening of credentials, network
boundaries, availability, retention, and incident response.
