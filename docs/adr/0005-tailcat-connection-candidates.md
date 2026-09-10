# Tailcat connections replace the hosted relay protocol

SSH and Tailcat share the existing Machine RPC Connector and identity confirmation. Cloud repurposes `organizationMachine` as encrypted backend-only connection metadata, scoped by Organization, Machine and current pairing; it is neither membership nor presence. This supersedes the protocol and slot decisions in 0001/0002 and the relay-hold completion condition in 0003, avoiding a second catalog or presence service.

The authenticated founder publishes its immutable candidate before completion. The current claim records the intended Machine as well as its public key; shared negotiation must confirm that Machine before transactional first-connect admission. Matching retries resume even after a lost completion response. Candidate absence and transport failure never authorize founder transfer or reset; reset remains unavailable until endpoint revocation can be confirmed.
