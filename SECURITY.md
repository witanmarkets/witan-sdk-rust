# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub:
**[Report a vulnerability](https://github.com/witanmarkets/witan-sdk-rust/security/advisories/new)**, in this
repository's **Security** tab.

Do not open a public issue, pull request or discussion for a security problem.

Please include:

- the crate version (`cargo tree -p witan-sdk`) and your Rust version
- what you did, what happened, and what you expected
- whether it is already public

You get an answer within a few days. Fixes ship as a new tag, and the advisory names the versions affected.

## Scope

In scope: the `witan-sdk` crate. Report problems with a WITAN origin (the hosted
service at witan.markets) through [witan.markets/contact](https://witan.markets/contact) or the same GitHub
form; they go to the same person.

Releases are published to crates.io from a tag on this repository by CI (trusted publishing); there is no
publish token in the repository.
