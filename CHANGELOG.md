# Changelog

Every release of the `witan-sdk` crate, newest first, grouped the way
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) groups them.

## 0.1.0 — 2026-10-23

### Added

- The first release: `search_error`, `search`, `read`, `datasets`, `dataset`, `dataset_data`, `query`,
  `quota` and `credits`, async on reqwest (rustls) and serde.
- `Client::builder()` with `api_key`, `base_url`, `user_agent`, `retries`, `timeout`; `WITAN_API_KEY` and
  `WITAN_BASE_URL` as fallbacks.
- `Error::Api` for every non-2xx answer and `Error::PaymentRequired` for a 402, with the price and the x402
  URL. Network errors, 429, 502, 503 and 504 are retried twice, honouring `Retry-After`.
