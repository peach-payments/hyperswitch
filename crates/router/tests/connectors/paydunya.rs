//! Connector tests for Paydunya.
//!
//! Paydunya is a mobile-money aggregator for francophone Africa (UEMOA + CEMAC).
//! The connector exposes a narrow surface compared to the card-based default
//! template:
//!
//! * `Authorize` is mapped to Paydunya's `softpay/<operator>` endpoints (MTN
//!   Benin/CI/Cameroun, Moov Benin/CI, Orange Money, Wave Senegal/CI, etc.).
//!   The request body shape is operator-specific and the operator is resolved
//!   from `(payment_method_type, billing.country)` — see [`PaydunyaOperator`]
//!   in the transformers module.
//! * `PSync` calls `GET checkout-invoice/confirm/{invoice_token}` and is keyed
//!   on the invoice token returned by the preprocessing flow.
//! * `Capture`, `Void`, `Execute`/`RSync` (refunds), `SetupMandate`, `Session`
//!   and `AccessTokenAuth` are intentionally **not** implemented upstream, so
//!   we do not exercise them here.
//!
//! Paydunya also requires a preprocessing call (`checkout-invoice/create`) to
//! happen before `Authorize`; that call returns the invoice token which the
//! `softpay/...` endpoint expects as `payment_token`. The current connector
//! test harness does not orchestrate that chain automatically — tests that
//! depend on a live sandbox roundtrip are therefore marked `#[ignore]` and are
//! intended to be run manually with valid `creds.json` entries.

use common_utils::pii::Email;
use hyperswitch_domain_models::{
    address::{Address, AddressDetails, PhoneDetails},
    payment_method_data::{MomoRedirection, PaymentMethodData, WalletData},
};
use hyperswitch_masking::Secret;
use router::types::{self, api, storage::enums};
use test_utils::connector_auth;

use crate::utils::{self, ConnectorActions};

#[derive(Clone, Copy)]
struct PaydunyaTest;
impl ConnectorActions for PaydunyaTest {}
impl utils::Connector for PaydunyaTest {
    fn get_data(&self) -> api::ConnectorData {
        use router::connector::Paydunya;
        utils::construct_connector_data_old(
            Box::new(Paydunya::new()),
            types::Connector::Paydunya,
            api::GetToken::Connector,
            None,
        )
    }

    fn get_auth_token(&self) -> types::ConnectorAuthType {
        utils::to_connector_auth_type(
            connector_auth::ConnectorAuthentication::new()
                .paydunya
                .expect("Missing connector authentication configuration")
                .into(),
        )
    }

    fn get_name(&self) -> String {
        "paydunya".to_string()
    }
}

static CONNECTOR: PaydunyaTest = PaydunyaTest {};

// ---------------------------------------------------------------------------
// Test data helpers
// ---------------------------------------------------------------------------

/// Build a `PaymentInfo` with the billing block Paydunya's SOFTPAY endpoints
/// require: full name, phone number and a country that can be resolved into
/// an operator. We default to MTN Benin (`country: BJ`).
fn mtn_benin_payment_info() -> Option<utils::PaymentInfo> {
    Some(utils::PaymentInfo {
        address: Some(types::PaymentAddress::new(
            None,
            Some(Address {
                address: Some(AddressDetails {
                    first_name: Some(Secret::new("Kossi".to_string())),
                    last_name: Some(Secret::new("Ahouanou".to_string())),
                    line1: Some(Secret::new("Rue 12.345".to_string())),
                    city: Some("Cotonou".to_string()),
                    zip: Some(Secret::new("00229".to_string())),
                    country: Some(api_models::enums::CountryAlpha2::BJ),
                    ..Default::default()
                }),
                phone: Some(PhoneDetails {
                    number: Some(Secret::new("90000000".to_string())),
                    country_code: Some("+229".to_string()),
                }),
                email: None,
            }),
            None,
            None,
        )),
        ..Default::default()
    })
}

/// Build an `Authorize` request for the MTN Benin SOFTPAY endpoint. Paydunya
/// uses `XOF` (West African CFA franc, zero-decimal) and amounts are passed
/// in the smallest unit.
fn mtn_benin_authorize_data() -> Option<types::PaymentsAuthorizeData> {
    Some(types::PaymentsAuthorizeData {
        amount: 1500,
        minor_amount: types::MinorUnit::new(1500),
        currency: enums::Currency::XOF,
        payment_method_data: PaymentMethodData::Wallet(WalletData::MomoRedirect(
            MomoRedirection {},
        )),
        payment_method_type: Some(enums::PaymentMethodType::Momo),
        confirm: true,
        email: Email::try_from("kossi.ahouanou@example.com".to_string()).ok(),
        router_return_url: Some("https://example.com/return".to_string()),
        webhook_url: Some("https://example.com/webhook".to_string()),
        ..utils::PaymentAuthorizeType::default().0
    })
}

// ---------------------------------------------------------------------------
// Authorize / SOFTPAY
// ---------------------------------------------------------------------------

/// The SOFTPAY authorize body needs `payment_token`, which the connector
/// expects to find on `RouterData.preprocessing_id` (populated by the
/// preceding `checkout-invoice/create` preprocessing call). When that
/// chain hasn't been run, `PaydunyaPaymentsRequest::try_from` bails with
/// `MissingConnectorRelatedTransactionID`.
///
/// This test pins that contract so a regression that silently drops the
/// invoice token (e.g. by removing the `preprocessing_id` propagation in
/// the Authorize flow) shows up immediately rather than hitting Paydunya
/// with a malformed request.
#[actix_web::test]
async fn should_fail_authorize_without_preprocessing_id() {
    let result = CONNECTOR
        .authorize_payment(mtn_benin_authorize_data(), mtn_benin_payment_info())
        .await;

    let err = result.expect_err("authorize without preprocessing_id must fail");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("payment_token")
            || rendered.contains("MissingConnectorRelatedTransactionID"),
        "expected MissingConnectorRelatedTransactionID error, got: {rendered}"
    );
}

/// Happy-path SOFTPAY authorize against the Paydunya sandbox.
///
/// Marked `#[ignore]` because it requires:
///   1. valid `paydunya` credentials in `connector_auth.toml`, and
///   2. a sandbox-side preprocessing roundtrip that the connector test
///      harness does not currently orchestrate (it would need the harness
///      to thread the `preprocessing_id` from `checkout-invoice/create`
///      into the subsequent `Authorize` `RouterData`).
///
/// Run manually with:
/// ```bash
/// cargo test --package router --test connectors paydunya::should_authorize_mtn_benin -- --ignored
/// ```
#[actix_web::test]
#[ignore = "requires Paydunya sandbox credentials and preprocessing chaining"]
async fn should_authorize_mtn_benin() {
    let response = CONNECTOR
        .authorize_payment(mtn_benin_authorize_data(), mtn_benin_payment_info())
        .await
        .expect("Authorize payment response");
    // SOFTPAY returns `processing` while the payer validates on their phone;
    // the IPN webhook then drives the attempt to `Charged`. Either status is
    // acceptable here depending on how fast the sandbox auto-confirms.
    assert!(
        matches!(
            response.status,
            enums::AttemptStatus::Authorizing | enums::AttemptStatus::Charged,
        ),
        "expected Authorizing or Charged, got {:?}",
        response.status,
    );
}

// ---------------------------------------------------------------------------
// PSync
// ---------------------------------------------------------------------------

/// Calling `GET checkout-invoice/confirm/{invoice_token}` against an unknown
/// token returns `response_code: "404"` / `status: "failed"` from Paydunya,
/// which the connector maps to [`enums::AttemptStatus::Failure`]. This test
/// exercises the URL builder + response parser without needing a live
/// transaction, so it's the closest thing to a CI-friendly smoke test.
///
/// Marked `#[ignore]` because it still needs sandbox credentials to actually
/// reach Paydunya — but the assertion below is correct and the test is safe
/// to run any time those credentials are available.
#[actix_web::test]
#[ignore = "requires Paydunya sandbox credentials"]
async fn should_sync_unknown_invoice_returns_failure() {
    let response = CONNECTOR
        .sync_payment(
            Some(types::PaymentsSyncData {
                connector_transaction_id: types::ResponseId::ConnectorTransactionId(
                    "test_unknown_invoice_token".to_string(),
                ),
                ..Default::default()
            }),
            mtn_benin_payment_info(),
        )
        .await
        .expect("PSync response");
    assert_eq!(response.status, enums::AttemptStatus::Failure);
}
