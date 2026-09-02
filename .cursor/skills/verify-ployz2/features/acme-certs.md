# ACME certificates

HTTPS Ingress Hostnames obtain a certificate through ACME. Informing tests use a fake CA (`ployz-testkit` `FakeCa`). Real Let's Encrypt needs a public IP and a hostname that already points at this Cluster.

## Sub-features

- `acme-https` Compose HTTPS `x-ports` (for example `app.example.com:8443:8080/https`) or an HTTPS hostname on a service.
- `acme-warn` if the hostname does not resolve to this Cluster, Deploy warns `A certificate cannot be issued until it points at this Cluster.`
- `acme-issue` once DNS points at the Machine, the Ingress Proxy serves a cert from the configured CA.

## How to get to it (user POV)

User deploys an HTTPS hostname, points DNS at the Cluster, waits for the proxy to obtain a cert, then `curl --https` that name.

## Driving it with helpers

Preconditions:

- Participating Machine plus either a fake ACME directory the daemon will use, or a public hostname and reachable port 80 for HTTP-01.

- **Warning only.** Deploy an HTTPS hostname that does not resolve here. Proof: stderr contains `A certificate cannot be issued until it points at this Cluster.` That is not a issued cert.
- **Issue.** Informing path: `ployz/tests/certificates_cluster.rs::custom_https_hostname_obtains_a_certificate_from_a_fake_ca` via `scripts/run-layer3-tests.sh`. User CLI on that Cluster: `ployz ingress deploy` then curl HTTPS.
- **Rung.** Informing as above. TSV rung 5 is `gap`.
- **Skip.** No fake CA hooked into this daemon, and no public ACME hostname. Do not call the warning line a successful issuance.

## Gotchas

- Hosted DNS (`ployz dns reserve`) can mint a cluster domain; it still is not ACME by itself.
- Mocks belong at the CA / DNS production boundary, not around `ployzd`.
