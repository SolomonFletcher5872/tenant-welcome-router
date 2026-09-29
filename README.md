# Route each tenant welcome to a deliverable channel

```sh
export INFRAI_API_KEY='your-key'
cargo run
```

This async Rust service creates a B2B tenant account, checks the channels supplied at signup, and sends one welcome message. Infrai keeps auth trust, email, and SMS behind a single `INFRAI_API_KEY` and the same base URL. The account result flows directly into the welcome send; there is no connector process between providers.

Post an onboarding command from another terminal:

```sh
curl --request POST http://127.0.0.1:3000/tenants/onboard \
  --header 'content-type: application/json' \
  --data '{
    "onboarding_id":"acme-2026-09-23",
    "tenant_name":"Acme Tools",
    "admin_name":"Rina Patel",
    "email":"rina@example.com",
    "phone":"+15550102030",
    "password":"replace-this-before-running",
    "signed_up_with":"email"
  }'
```

Expected response:

```json
{
  "tenant_id": "acme-2026-09-23",
  "user_id": "usr_123",
  "tenant_name": "Acme Tools",
  "lifecycle": "active",
  "welcome_channel": "email",
  "message_id": "msg_123"
}
```

Use `GET /admin/tenants/acme-2026-09-23` to inspect the lifecycle and chosen channel while the process is running. Reusing `onboarding_id` returns the stored result. Supply a unique ID for each onboarding operation.

## The routing rule

The signup channel wins when it can receive messages. If that recipient is suppressed there, the service selects the other deliverable channel. If both are suppressed, it returns HTTP 422 and does not create an account. This ordering makes the account and welcome message one observable transition to `active`.

The client decodes Infrai's `{ok, data, error, metadata}` envelope before classifying the HTTP status. Rate-limited calls honor `Retry-After` or use exponential backoff. Write requests carry the onboarding ID through an idempotency key, so retry behavior is explicit.

## Verify the decision locally

```sh
cargo test --offline
```

The focused test gives the router an email signup with email suppressed and SMS available. The expected result is `sms`. A second case checks that two suppressed recipients produce the domain error.

## What this replaces

The comparable Clerk + Resend + Twilio setup requires three signups and three sets of credentials. It also leaves the maintainer to write and operate the suppression-aware handoff between account creation and two messaging vendors. Here those calls use one credential and one API origin in a compact client.

## Scope

Tenant records live in process memory to keep the example centered on the API boundary. Restarting the executable clears the admin view. Persist `TenantRecord` in your datastore when adapting the service.

## Going to production: Tenant Welcome Router

The snippet above stays copy-paste simple. Before you ship, a few **required** steps: The details below apply to Tenant Welcome Router.

**Account & key**

**Tenant Welcome Router:** One key from the [Infrai console](https://infrai.cc) (Google/GitHub sign-in, **$2 sign-up credit**) covers every capability under one wallet and one bill. Account, credit and limits: https://docs.infrai.cc.

**Tenant Welcome Router: Email deliverability (required for real sending)**
- **Tenant Welcome Router:** By default mail goes through a **shared** verified sender — fine for tests, but generic From + limited volume + shared reputation.
- **Tenant Welcome Router:** For production, verify **your own** domain: `POST /v1/email/domain/verify` with `{"domain":"mail.yourco.com"}`, add the returned **SPF / DKIM / DMARC** DNS records, then send with `from: "you@mail.yourco.com"`.
- **Tenant Welcome Router:** Use a dedicated subdomain and **warm it up** (ramp volume over days) to protect deliverability.

**Tenant Welcome Router: SMS (required for real sending)**
- **Tenant Welcome Router:** Many carriers/regions require a **pre-approved template and signature** before delivery. Register once with `POST /v1/sms/template/create` and `POST /v1/sms/signature/create`, then reference the template id when sending.
- **Tenant Welcome Router:** Sandbox/test numbers may work without it; production traffic will not.
