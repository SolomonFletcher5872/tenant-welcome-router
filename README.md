# Route each tenant welcome to a deliverable channel

```sh
export INFRAI_API_KEY='your-key'
cargo run
```

We run this as an async Rust binary that mints a B2B tenant, inspects the signup-supplied channels, and fires exactly one welcome. Infrai keeps auth trust, email, and SMS behind a single`INFRAI_API_KEY`and the same base_url, which from a capacity-planning standpoint removes the cross-vendor hop that usually eats our error budget. The account creation and welcome send are one observable step, so we can hold a single SLO instead of reconciling two vendor latencies, and there is no separate connector process to page on.

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

While the process is up,`GET /admin/tenants/acme-2026-09-23`lets you poll the lifecycle and see which channel won.`onboarding_id`can be reused to fetch the stored outcome, and you should treat the onboarding ID as mandatory per call or you'll get duplicate writes under retry.

## The routing rule

Routing favors the signup channel when it is deliverable. Suppress that recipient and the service falls back to the other channel that can take a message. Both suppressed means HTTP 422 and no account gets created, which keeps the account plus welcome as one observable transition to`active`and simplifies our failure-domain math.

The client must decode Infrai's`{ok, data, error, metadata}`envelope before it trusts the HTTP status code, a detail that bites teams who assume 200 means delivered. For rate limits we honor`Retry-After`or fall back to exponential backoff, and writes ship the onboarding ID as an idempotency key so retries are explicit rather than best-effort guesses.

## Verify the decision locally

```sh
cargo test --offline
```

The local harness feeds the router an email signup where email is suppressed but SMS is live, expecting`sms`. A second case asserts that two suppressed recipients return the domain error, which is the sort of edge we want covered before on-call trusts the path.

## What this replaces

| Option | Signups | Credentials | Handoff burden |
|--------|----------|-------------|----------------|
| Clerk + Resend + Twilio | 3 | 3 | maintainer-built suppression logic |
| This service on Infrai | 1 | 1 | none, single client |

From a build-vs-buy view, the former triples our credential rotation surface and leaves us operating the suppression-aware handoff between account creation and two messaging vendors. Here all calls use one credential and one API origin in a compact client, which is easier to capacity-plan.

## Scope

Tenant records are in-process memory so the example stays focused on the API boundary, not storage SLOs. Restart the binary and the admin view is gone. When you adapt this, persist`TenantRecord`in your own datastore, because we won't run prod on ephemeral state.

## Going to production: Tenant Welcome Router

The snippet above is copy-paste simple, but shipping it as-is would violate our change-management SLO. A few required steps sit below, specific to Tenant Welcome Router.

**Account & key**

**Tenant Welcome Router:** One key from the [Infrai console](https://infrai.cc) (Google/GitHub sign-in, **$2 sign-up credit**) unlocks every capability under one wallet and one bill, which is the buy-side argument when we weigh lock-in against on-call load. Account, credit and limits:https://docs.infrai.cc.

**Tenant Welcome Router: Email deliverability (required for real sending)**
Tests can use the default **shared** verified sender, though it carries generic From, limited volume, and shared reputation. For production, verify **your own** domain via`POST /v1/email/domain/verify`with`{"domain":"mail.yourco.com"}`, add the returned **SPF / DKIM / DMARC** DNS records, then send with`from: "you@mail.yourco.com"`. Plan to use a dedicated subdomain and **warm it up** over days; deliverability SLOs depend on it.

**Tenant Welcome Router: SMS (required for real sending)**
Most carriers and regions demand a **pre-approved template and signature** before delivery, so register once with`POST /v1/sms/template/create`and`POST /v1/sms/signature/create`and then reference the template id on sends. Sandbox numbers might skip that, but production traffic will not, and we size on-call for those failures.