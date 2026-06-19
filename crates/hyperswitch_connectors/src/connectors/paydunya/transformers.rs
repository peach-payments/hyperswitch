use common_enums::enums;
use common_utils::{pii::Email, types::MinorUnit};
use error_stack::ResultExt;
use hyperswitch_domain_models::{
    payment_method_data::{PaymentMethodData, WalletData},
    router_data::{ConnectorAuthType, ErrorResponse, RouterData},
    router_flow_types::{
        refunds::{Execute, RSync},
        Authorize, PSync,
    },
    router_request_types::{PaymentsAuthorizeData, PaymentsSyncData, ResponseId},
    router_response_types::{PaymentsResponseData, RefundsResponseData},
    types::{CreateOrderRouterData, PaymentsAuthorizeRouterData, RefundsRouterData},
};
use hyperswitch_interfaces::{
    consts::{NO_ERROR_CODE, NO_ERROR_MESSAGE},
    errors,
};
use hyperswitch_masking::Secret;
use serde::{Deserialize, Serialize};

use crate::{
    types::{CreateOrderResponseRouterData, RefundsResponseRouterData, ResponseRouterData},
    utils::{
        to_connector_meta_from_secret, AddressDetailsData, PaymentsAuthorizeRequestData,
        RouterData as _,
    },
};

pub mod paydunya_constants {
    pub const PAYDUNYA_MASTER_KEY: &str = "PAYDUNYA-MASTER-KEY";
    pub const PAYDUNYA_PRIVATE_KEY: &str = "PAYDUNYA-PRIVATE-KEY";
    pub const PAYDUNYA_TOKEN: &str = "PAYDUNYA-TOKEN";
}

//TODO: Fill the struct with respective fields
pub struct PaydunyaRouterData<T> {
    pub amount: MinorUnit, // The type of amount that a connector accepts, for example, String, i64, f64, etc.
    pub router_data: T,
}

impl<T> From<(MinorUnit, T)> for PaydunyaRouterData<T> {
    fn from((amount, item): (MinorUnit, T)) -> Self {
        //Todo :  use utils to convert the amount to the type of amount that a connector accepts
        Self {
            amount,
            router_data: item,
        }
    }
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Serialize, Default)]
pub struct PaydunyaPreprocessingRequest {
    pub invoice: Invoice,
    pub store: Store,
    pub actions: Actions,
}

#[derive(Debug, Serialize, Default)]
pub struct Invoice {
    pub total_amount: MinorUnit,
}

#[derive(Debug, Serialize, Default)]
pub struct Store {
    pub name: String,
}

#[derive(Debug, Serialize, Default)]
pub struct Actions {
    pub callback_url: String,
    pub return_url: String,
}

/// Paydunya requires a store name on every invoice, which we surface
/// from the merchant's business/profile name sent to Paydunya as `store.name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaydunyaConnectorMetadataObject {
    pub store_name: String,
}

impl TryFrom<Option<&common_utils::pii::SecretSerdeValue>> for PaydunyaConnectorMetadataObject {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        meta_data: Option<&common_utils::pii::SecretSerdeValue>,
    ) -> Result<Self, Self::Error> {
        to_connector_meta_from_secret::<Self>(meta_data.cloned()).change_context(
            errors::ConnectorError::InvalidConnectorConfig {
                config: "metadata.store_name",
            },
        )
    }
}

impl TryFrom<&CreateOrderRouterData> for PaydunyaPreprocessingRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(item: &CreateOrderRouterData) -> Result<Self, Self::Error> {
        let callback_url = item.request.webhook_url.clone().unwrap_or_default();
        let return_url = item.request.router_return_url.clone().unwrap_or_default();

        let metadata =
            PaydunyaConnectorMetadataObject::try_from(item.connector_meta_data.as_ref())?;

        Ok(Self {
            invoice: Invoice {
                total_amount: item.request.minor_amount,
            },
            store: Store {
                name: metadata.store_name,
            },
            actions: Actions {
                callback_url,
                return_url,
            },
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaydunyaPaymentsPreProcessingResponse {
    pub response_code: String,
    pub response_text: String,
    // `description` and `token` are only present on a successful invoice
    // creation (`response_code` == "00"). On failure Paydunya still returns
    // HTTP 200 with just `response_code`/`response_text`, so these must be
    // optional to avoid a deserialization failure that masks the real error.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

impl TryFrom<CreateOrderResponseRouterData<PaydunyaPaymentsPreProcessingResponse>>
    for CreateOrderRouterData
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: CreateOrderResponseRouterData<PaydunyaPaymentsPreProcessingResponse>,
    ) -> Result<Self, Self::Error> {
        let response = if item.response.response_code == "00" {
            match item.response.token.clone() {
                Some(order_id) => Ok(PaymentsResponseData::PaymentsCreateOrderResponse {
                    order_id,
                    session_token: None,
                }),
                None => Err(ErrorResponse {
                    status_code: item.http_code,
                    code: item.response.response_code.clone(),
                    message: item.response.response_text.clone(),
                    reason: Some(
                        "Paydunya invoice creation succeeded but no token was returned".to_string(),
                    ),
                    attempt_status: None,
                    connector_transaction_id: None,
                    connector_response_reference_id: None,
                    network_advice_code: None,
                    network_decline_code: None,
                    network_error_message: None,
                    connector_metadata: None,
                }),
            }
        } else {
            Err(ErrorResponse {
                status_code: item.http_code,
                code: item.response.response_code.clone(),
                message: item.response.response_text.clone(),
                reason: item
                    .response
                    .description
                    .clone()
                    .or_else(|| Some(item.response.response_text.clone())),
                attempt_status: None,
                connector_transaction_id: None,
                connector_response_reference_id: None,
                network_advice_code: None,
                network_decline_code: None,
                network_error_message: None,
                connector_metadata: None,
            })
        };

        Ok(Self {
            response,
            ..item.data
        })
    }
}

/// Mobile-money / wallet operator served by Paydunya's SOFTPAY API.
///
/// Each operator has its own endpoint and its own request body shape (field
/// names are prefixed with the operator name, e.g. `mtn_benin_*`, `wave_ci_*`).
/// New operators can be added by extending this enum and the
/// [`PaydunyaPaymentsRequest`] variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaydunyaOperator {
    MtnBenin,
    MtnCi,
    MtnCameroun,
    MoovBenin,
    MoovCi,
    MoovMali,
    MoovTogo,
    MoovBurkina,
    OrangeMoneyCi,
    OrangeMoneySenegal,
    OrangeMoneyMali,
    OrangeMoneyBurkina,
    WaveSenegal,
    WaveCi,
    FreeMoneySenegal,
    ExpressoSenegal,
    DjamoCi,
    DjamoSn,
    TmoneyTogo,
    WizallSenegal,
}

impl PaydunyaOperator {
    /// Path appended to the Paydunya base URL for the SOFTPAY call.
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::MtnBenin => "softpay/mtn-benin",
            Self::MtnCi => "softpay/mtn-ci",
            Self::MtnCameroun => "softpay/mtn-cameroun",
            Self::MoovBenin => "softpay/moov-benin",
            Self::MoovCi => "softpay/moov-ci",
            Self::MoovMali => "softpay/moov-mali",
            Self::MoovTogo => "softpay/moov-togo",
            Self::MoovBurkina => "softpay/moov-burkina",
            Self::OrangeMoneyCi => "softpay/orange-money-ci",
            Self::OrangeMoneySenegal => "softpay/new-orange-money-senegal",
            Self::OrangeMoneyMali => "softpay/orange-money-mali",
            Self::OrangeMoneyBurkina => "softpay/orange-money-burkina",
            Self::WaveSenegal => "softpay/wave-senegal",
            Self::WaveCi => "softpay/wave-ci",
            Self::FreeMoneySenegal => "softpay/free-money-senegal",
            Self::ExpressoSenegal => "softpay/expresso-senegal",
            Self::DjamoCi | Self::DjamoSn => "softpay/djamo",
            Self::TmoneyTogo => "softpay/t-money-togo",
            Self::WizallSenegal => "softpay/wizall-senegal",
        }
    }

    /// Value Paydunya expects in the `*_wallet_provider` field for operators
    /// that require it (currently the MTN family).
    pub fn wallet_provider(self) -> Option<&'static str> {
        match self {
            Self::MtnBenin => Some("MTNBENIN"),
            Self::MtnCi => Some("MTNCI"),
            Self::MtnCameroun => Some("MTNCAMEROUN"),
            _ => None,
        }
    }

    /// ISO-3166 alpha-2 (lowercased) discriminator the shared Djamo SOFTPAY
    /// endpoint uses to pick the regional account ("ci" / "sn"). `None` for
    /// every non-Djamo operator.
    pub fn djamo_code_country(self) -> Option<&'static str> {
        match self {
            Self::DjamoCi => Some("ci"),
            Self::DjamoSn => Some("sn"),
            _ => None,
        }
    }
}

impl TryFrom<&PaymentsAuthorizeRouterData> for PaydunyaOperator {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(item: &PaymentsAuthorizeRouterData) -> Result<Self, Self::Error> {
        // Each Paydunya operator is selected from
        // `(payment_method_type, billing.country)` — the payment-method-type
        // picks the carrier family (MTN MoMo / Moov Money / Wave / ...) and the
        // billing country picks the regional SOFTPAY endpoint within that
        // family. The country is therefore mandatory for routing on every
        // wallet, so we require it up front (errors with `MissingRequiredField`)
        // rather than guessing a region when it's absent.
        let pm_type = item.request.payment_method_type;
        let country = item.get_billing_country()?;

        match (pm_type, country) {
            // MTN family — typed as MoMo upstream
            (Some(enums::PaymentMethodType::Momo), enums::CountryAlpha2::BJ) => Ok(Self::MtnBenin),
            (Some(enums::PaymentMethodType::Momo), enums::CountryAlpha2::CI) => Ok(Self::MtnCi),
            (Some(enums::PaymentMethodType::Momo), enums::CountryAlpha2::CM) => {
                Ok(Self::MtnCameroun)
            }

            // Moov family
            (Some(enums::PaymentMethodType::MoovMoney), enums::CountryAlpha2::BJ) => {
                Ok(Self::MoovBenin)
            }
            (Some(enums::PaymentMethodType::MoovMoney), enums::CountryAlpha2::CI) => {
                Ok(Self::MoovCi)
            }
            (Some(enums::PaymentMethodType::MoovMoney), enums::CountryAlpha2::ML) => {
                Ok(Self::MoovMali)
            }
            (Some(enums::PaymentMethodType::MoovMoney), enums::CountryAlpha2::TG) => {
                Ok(Self::MoovTogo)
            }
            (Some(enums::PaymentMethodType::MoovMoney), enums::CountryAlpha2::BF) => {
                Ok(Self::MoovBurkina)
            }

            // Wave family
            (Some(enums::PaymentMethodType::Wave), enums::CountryAlpha2::SN) => {
                Ok(Self::WaveSenegal)
            }
            (Some(enums::PaymentMethodType::Wave), enums::CountryAlpha2::CI) => Ok(Self::WaveCi),

            // Orange Money family — Paydunya exposes one SOFTPAY endpoint per
            // country. We pick the regional operator off the billing country,
            // which the SOFTPAY API treats as authoritative.
            (Some(enums::PaymentMethodType::OrangeMoney), enums::CountryAlpha2::CI) => {
                Ok(Self::OrangeMoneyCi)
            }
            (Some(enums::PaymentMethodType::OrangeMoney), enums::CountryAlpha2::SN) => {
                Ok(Self::OrangeMoneySenegal)
            }
            (Some(enums::PaymentMethodType::OrangeMoney), enums::CountryAlpha2::ML) => {
                Ok(Self::OrangeMoneyMali)
            }
            (Some(enums::PaymentMethodType::OrangeMoney), enums::CountryAlpha2::BF) => {
                Ok(Self::OrangeMoneyBurkina)
            }

            // Djamo family — Côte d'Ivoire and Senegal share one SOFTPAY
            // endpoint (`softpay/djamo`); the regional account is picked via
            // the `code_country` field resolved off the billing country.
            (Some(enums::PaymentMethodType::Djamo), enums::CountryAlpha2::CI) => Ok(Self::DjamoCi),
            (Some(enums::PaymentMethodType::Djamo), enums::CountryAlpha2::SN) => Ok(Self::DjamoSn),

            // T-Money family — Togo is the only region Paydunya exposes for
            // T-Money. The country is still mandatory (resolved above), but any
            // value maps to the single `softpay/t-money-togo` endpoint.
            (Some(enums::PaymentMethodType::TMoney), _) => Ok(Self::TmoneyTogo),

            // Wizall family — Senegal is the only region Paydunya exposes for
            // Wizall Money. The country is still mandatory, but any value maps
            // to the single `softpay/wizall-senegal` endpoint.
            (Some(enums::PaymentMethodType::Wizall), _) => Ok(Self::WizallSenegal),

            // Expresso family — Senegal is the only region Paydunya exposes for
            // Expresso. The country is still mandatory, but any value maps to
            // the single `softpay/expresso-senegal` endpoint.
            (Some(enums::PaymentMethodType::Expresso), _) => Ok(Self::ExpressoSenegal),

            // Free Money family — Senegal is the only region Paydunya exposes
            // for Free Money. The country is still mandatory, but any value
            // maps to the single `softpay/free-money-senegal` endpoint.
            (Some(enums::PaymentMethodType::FreeMoney), _) => Ok(Self::FreeMoneySenegal),

            _ => Err(errors::ConnectorError::NotImplemented(format!(
                "Paydunya operator resolution for payment_method_type={pm_type:?} country={country:?}"
            ))
            .into()),
        }
    }
}

/// Authorize / SOFTPAY request body. Each variant matches one Paydunya
/// operator endpoint and serialises to the exact JSON shape that operator
/// expects. The enum is `untagged` so the variant is selected purely by the
/// `PaydunyaOperator` resolved from the router data.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum PaydunyaPaymentsRequest {
    MtnBenin(PaydunyaMtnBeninRequest),
    MtnCi(PaydunyaMtnCiRequest),
    MtnCameroun(PaydunyaMtnCamerounRequest),
    MoovBenin(PaydunyaMoovBeninRequest),
    MoovCi(PaydunyaMoovCiRequest),
    MoovMali(PaydunyaMoovMaliRequest),
    MoovTogo(PaydunyaMoovTogoRequest),
    MoovBurkina(PaydunyaMoovBurkinaRequest),
    OrangeMoneyCi(PaydunyaOrangeMoneyCiRequest),
    OrangeMoneySenegal(PaydunyaOrangeMoneySenegalRequest),
    OrangeMoneyMali(PaydunyaOrangeMoneyMaliRequest),
    OrangeMoneyBurkina(PaydunyaOrangeMoneyBurkinaRequest),
    WaveSenegal(PaydunyaWaveSenegalRequest),
    WaveCi(PaydunyaWaveCiRequest),
    FreeMoneySenegal(PaydunyaFreeMoneySenegalRequest),
    ExpressoSenegal(PaydunyaExpressoSenegalRequest),
    Djamo(PaydunyaDjamoRequest),
    TmoneyTogo(PaydunyaTmoneyTogoRequest),
    WizallSenegal(PaydunyaWizallSenegalRequest),
}

#[derive(Debug, Serialize)]
pub struct PaydunyaMtnBeninRequest {
    pub mtn_benin_customer_fullname: Secret<String>,
    pub mtn_benin_email: Email,
    pub mtn_benin_phone_number: Secret<String>,
    pub mtn_benin_wallet_provider: &'static str,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaMtnCiRequest {
    pub mtn_ci_customer_fullname: Secret<String>,
    pub mtn_ci_email: Email,
    pub mtn_ci_phone_number: Secret<String>,
    pub mtn_ci_wallet_provider: &'static str,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaMtnCamerounRequest {
    pub mtn_cameroun_customer_fullname: Secret<String>,
    pub mtn_cameroun_email: Email,
    pub mtn_cameroun_phone_number: Secret<String>,
    pub mtn_cameroun_wallet_provider: &'static str,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaMoovBeninRequest {
    pub moov_benin_customer_fullname: Secret<String>,
    pub moov_benin_email: Email,
    pub moov_benin_phone_number: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaMoovCiRequest {
    pub moov_ci_customer_fullname: Secret<String>,
    pub moov_ci_email: Email,
    pub moov_ci_phone_number: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaMoovMaliRequest {
    pub moov_ml_customer_fullname: Secret<String>,
    pub moov_ml_email: Email,
    pub moov_ml_phone_number: Secret<String>,
    pub moov_ml_customer_address: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaMoovTogoRequest {
    pub moov_togo_customer_fullname: Secret<String>,
    pub moov_togo_email: Email,
    pub moov_togo_customer_address: Secret<String>,
    pub moov_togo_phone_number: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaMoovBurkinaRequest {
    // Burkina's Moov endpoint expects a camelCased full-name key and an
    // operator-prefixed `*_payment_token` (unlike the generic `payment_token`
    // used by the Mali/Togo endpoints).
    #[serde(rename = "moov_burkina_faso_fullName")]
    pub moov_burkina_faso_full_name: Secret<String>,
    pub moov_burkina_faso_email: Email,
    pub moov_burkina_faso_phone_number: Secret<String>,
    pub moov_burkina_faso_payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaOrangeMoneyCiRequest {
    pub orange_money_ci_customer_fullname: Secret<String>,
    pub orange_money_ci_email: Email,
    pub orange_money_ci_phone_number: Secret<String>,
    /// One-time code generated by the payer via the `#144*82#` USSD flow.
    pub orange_money_ci_otp: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaOrangeMoneySenegalRequest {
    pub customer_name: Secret<String>,
    pub customer_email: Email,
    pub phone_number: Secret<String>,
    pub invoice_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaOrangeMoneyMaliRequest {
    pub orange_money_mali_customer_fullname: Secret<String>,
    pub orange_money_mali_email: Email,
    pub orange_money_mali_phone_number: Secret<String>,
    pub orange_money_mali_customer_address: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaOrangeMoneyBurkinaRequest {
    pub name_bf: Secret<String>,
    pub email_bf: Email,
    pub phone_bf: Secret<String>,
    /// One-time code generated by the payer on their Orange Money app /
    /// USSD menu before the SOFTPAY call.
    pub otp_code: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaWaveSenegalRequest {
    #[serde(rename = "wave_senegal_fullName")]
    pub wave_senegal_full_name: Secret<String>,
    pub wave_senegal_email: Email,
    pub wave_senegal_phone: Secret<String>,
    pub wave_senegal_payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaWaveCiRequest {
    #[serde(rename = "wave_ci_fullName")]
    pub wave_ci_full_name: Secret<String>,
    pub wave_ci_email: Email,
    pub wave_ci_phone: Secret<String>,
    pub wave_ci_payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaFreeMoneySenegalRequest {
    pub customer_name: Secret<String>,
    pub customer_email: Email,
    pub phone_number: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaExpressoSenegalRequest {
    #[serde(rename = "expresso_sn_fullName")]
    pub expresso_sn_full_name: Secret<String>,
    pub expresso_sn_email: Email,
    pub expresso_sn_phone: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaDjamoRequest {
    // Djamo's SOFTPAY endpoint expects a camelCased full-name key and the
    // operator-prefixed `djamo_payment_token` (not the generic `payment_token`).
    #[serde(rename = "djamo_fullName")]
    pub djamo_full_name: Secret<String>,
    pub djamo_email: Email,
    pub djamo_phone: Secret<String>,
    /// Lowercased ISO-3166 alpha-2 country ("ci" / "sn") selecting the regional
    /// account on the shared `softpay/djamo` endpoint.
    pub code_country: &'static str,
    pub djamo_payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaTmoneyTogoRequest {
    pub name_t_money: Secret<String>,
    pub email_t_money: Email,
    pub phone_t_money: Secret<String>,
    pub payment_token: String,
}

#[derive(Debug, Serialize)]
pub struct PaydunyaWizallSenegalRequest {
    // Wizall Senegal's SOFTPAY endpoint expects a camelCased full-name key and
    // an operator-prefixed `wizall_senegal_payment_token` (mirroring the Wave
    // Senegal layout) rather than the generic `payment_token`.
    #[serde(rename = "wizall_senegal_fullName")]
    pub wizall_senegal_full_name: Secret<String>,
    pub wizall_senegal_email: Email,
    pub wizall_senegal_phone: Secret<String>,
    pub wizall_senegal_payment_token: String,
}

struct CommonAuthorizeFields {
    full_name: Secret<String>,
    email: Email,
    phone_number: Secret<String>,
    payment_token: String,
}

/// Extract an optional Orange Money OTP off the payment method data.
///
/// Some Orange Money SOFTPAY endpoints (Côte d'Ivoire, Burkina Faso) require
/// the payer to generate a one-time code on their handset before confirming
/// the payment. Hyperswitch transports that OTP through the typed
/// [`WalletData::OrangeMoneyRedirect`] variant. Any other wallet shape — or a
/// non-wallet payment method — is treated as "no OTP supplied" so the caller
/// can decide whether to fall back to `MissingRequiredField`.
fn extract_orange_money_otp(payment_method_data: &PaymentMethodData) -> Option<Secret<String>> {
    match payment_method_data {
        PaymentMethodData::Wallet(WalletData::OrangeMoneyRedirect(data)) => data.otp.clone(),
        _ => None,
    }
}

fn billing_customer_address(
    item: &PaydunyaRouterData<&PaymentsAuthorizeRouterData>,
) -> Result<Secret<String>, error_stack::Report<errors::ConnectorError>> {
    let address = item
        .router_data
        .get_optional_billing()
        .and_then(|b| b.address.as_ref())
        .ok_or(errors::ConnectorError::MissingRequiredField {
            field_name: "billing.address",
        })?;
    address
        .get_city()
        .map(|city| Secret::new(city.clone()))
        .or_else(|_| address.get_line1().cloned())
}

impl CommonAuthorizeFields {
    fn try_from_router_data(
        item: &PaydunyaRouterData<&PaymentsAuthorizeRouterData>,
    ) -> Result<Self, error_stack::Report<errors::ConnectorError>> {
        let router_data = item.router_data;

        // The SOFTPAY payment_token must be the invoice token returned by the
        // checkout-invoice/create order-create call, which the order-create
        // pre-task stashes on `request.order_id`.
        let payment_token = router_data.request.order_id.clone().ok_or(
            errors::ConnectorError::MissingConnectorRelatedTransactionID {
                id: "payment_token (paydunya invoice token from order create)".to_string(),
            },
        )?;

        let billing = router_data
            .get_optional_billing()
            .and_then(|b| b.address.as_ref());

        let full_name = billing
            .ok_or(errors::ConnectorError::MissingRequiredField {
                field_name: "billing.address",
            })?
            .get_full_name()?;

        let phone_number = router_data.get_optional_billing_phone_number().ok_or(
            errors::ConnectorError::MissingRequiredField {
                field_name: "payment_method_data.billing.phone.number",
            },
        )?;
        let email = router_data.request.get_email()?;

        Ok(Self {
            full_name,
            email,
            phone_number,
            payment_token,
        })
    }
}

impl TryFrom<&PaydunyaRouterData<&PaymentsAuthorizeRouterData>> for PaydunyaPaymentsRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: &PaydunyaRouterData<&PaymentsAuthorizeRouterData>,
    ) -> Result<Self, Self::Error> {
        let operator = PaydunyaOperator::try_from(item.router_data)?;
        let common = CommonAuthorizeFields::try_from_router_data(item)?;

        let request = match operator {
            PaydunyaOperator::MtnBenin => Self::MtnBenin(PaydunyaMtnBeninRequest {
                mtn_benin_customer_fullname: common.full_name,
                mtn_benin_email: common.email,
                mtn_benin_phone_number: common.phone_number,
                mtn_benin_wallet_provider: operator.wallet_provider().unwrap_or("MTNBENIN"),
                payment_token: common.payment_token,
            }),
            PaydunyaOperator::MtnCi => Self::MtnCi(PaydunyaMtnCiRequest {
                mtn_ci_customer_fullname: common.full_name,
                mtn_ci_email: common.email,
                mtn_ci_phone_number: common.phone_number,
                mtn_ci_wallet_provider: operator.wallet_provider().unwrap_or("MTNCI"),
                payment_token: common.payment_token,
            }),
            PaydunyaOperator::MtnCameroun => Self::MtnCameroun(PaydunyaMtnCamerounRequest {
                mtn_cameroun_customer_fullname: common.full_name,
                mtn_cameroun_email: common.email,
                mtn_cameroun_phone_number: common.phone_number,
                mtn_cameroun_wallet_provider: operator.wallet_provider().unwrap_or("MTNCAMEROUN"),
                payment_token: common.payment_token,
            }),
            PaydunyaOperator::MoovBenin => Self::MoovBenin(PaydunyaMoovBeninRequest {
                moov_benin_customer_fullname: common.full_name,
                moov_benin_email: common.email,
                moov_benin_phone_number: common.phone_number,
                payment_token: common.payment_token,
            }),
            PaydunyaOperator::MoovCi => Self::MoovCi(PaydunyaMoovCiRequest {
                moov_ci_customer_fullname: common.full_name,
                moov_ci_email: common.email,
                moov_ci_phone_number: common.phone_number,
                payment_token: common.payment_token,
            }),
            PaydunyaOperator::MoovMali => Self::MoovMali(PaydunyaMoovMaliRequest {
                moov_ml_customer_fullname: common.full_name,
                moov_ml_email: common.email,
                moov_ml_phone_number: common.phone_number,
                moov_ml_customer_address: billing_customer_address(item)?,
                payment_token: common.payment_token,
            }),
            PaydunyaOperator::MoovTogo => Self::MoovTogo(PaydunyaMoovTogoRequest {
                moov_togo_customer_fullname: common.full_name,
                moov_togo_email: common.email,
                moov_togo_customer_address: billing_customer_address(item)?,
                moov_togo_phone_number: common.phone_number,
                payment_token: common.payment_token,
            }),
            PaydunyaOperator::MoovBurkina => Self::MoovBurkina(PaydunyaMoovBurkinaRequest {
                moov_burkina_faso_full_name: common.full_name,
                moov_burkina_faso_email: common.email,
                moov_burkina_faso_phone_number: common.phone_number,
                moov_burkina_faso_payment_token: common.payment_token,
            }),
            PaydunyaOperator::OrangeMoneyCi => {
                // Côte d'Ivoire requires the payer to generate an OTP via
                // USSD `#144*82#` (option 2) before confirming. The merchant
                // must collect that code and ship it through the typed
                // `WalletData::OrangeMoneyRedirect { otp }` transport — we
                // fail the call outright if it is not supplied rather than
                // hitting Paydunya with an empty `orange_money_ci_otp`.
                let otp = extract_orange_money_otp(&item.router_data.request.payment_method_data)
                    .ok_or(errors::ConnectorError::MissingRequiredField {
                    field_name: "payment_method_data.wallet.orange_money_redirect.otp",
                })?;
                Self::OrangeMoneyCi(PaydunyaOrangeMoneyCiRequest {
                    orange_money_ci_customer_fullname: common.full_name,
                    orange_money_ci_email: common.email,
                    orange_money_ci_phone_number: common.phone_number,
                    orange_money_ci_otp: otp,
                    payment_token: common.payment_token,
                })
            }
            PaydunyaOperator::OrangeMoneyBurkina => {
                // Burkina Faso's SOFTPAY endpoint takes an `otp_code` the
                // payer obtains from their Orange Money app/USSD menu.
                // Same transport as Côte d'Ivoire above; we reject early
                // when the merchant hasn't shipped the code through.
                let otp = extract_orange_money_otp(&item.router_data.request.payment_method_data)
                    .ok_or(errors::ConnectorError::MissingRequiredField {
                    field_name: "payment_method_data.wallet.orange_money_redirect.otp",
                })?;
                Self::OrangeMoneyBurkina(PaydunyaOrangeMoneyBurkinaRequest {
                    name_bf: common.full_name,
                    email_bf: common.email,
                    phone_bf: common.phone_number,
                    otp_code: otp,
                    payment_token: common.payment_token,
                })
            }
            PaydunyaOperator::OrangeMoneySenegal => {
                Self::OrangeMoneySenegal(PaydunyaOrangeMoneySenegalRequest {
                    customer_name: common.full_name,
                    customer_email: common.email,
                    phone_number: common.phone_number,
                    invoice_token: common.payment_token,
                })
            }
            PaydunyaOperator::OrangeMoneyMali => {
                Self::OrangeMoneyMali(PaydunyaOrangeMoneyMaliRequest {
                    orange_money_mali_customer_fullname: common.full_name,
                    orange_money_mali_email: common.email,
                    orange_money_mali_phone_number: common.phone_number,
                    orange_money_mali_customer_address: billing_customer_address(item)?,
                    payment_token: common.payment_token,
                })
            }
            PaydunyaOperator::WaveSenegal => Self::WaveSenegal(PaydunyaWaveSenegalRequest {
                wave_senegal_full_name: common.full_name,
                wave_senegal_email: common.email,
                wave_senegal_phone: common.phone_number,
                wave_senegal_payment_token: common.payment_token,
            }),
            PaydunyaOperator::WaveCi => Self::WaveCi(PaydunyaWaveCiRequest {
                wave_ci_full_name: common.full_name,
                wave_ci_email: common.email,
                wave_ci_phone: common.phone_number,
                wave_ci_payment_token: common.payment_token,
            }),
            PaydunyaOperator::FreeMoneySenegal => {
                Self::FreeMoneySenegal(PaydunyaFreeMoneySenegalRequest {
                    customer_name: common.full_name,
                    customer_email: common.email,
                    phone_number: common.phone_number,
                    payment_token: common.payment_token,
                })
            }
            PaydunyaOperator::ExpressoSenegal => {
                Self::ExpressoSenegal(PaydunyaExpressoSenegalRequest {
                    expresso_sn_full_name: common.full_name,
                    expresso_sn_email: common.email,
                    expresso_sn_phone: common.phone_number,
                    payment_token: common.payment_token,
                })
            }
            PaydunyaOperator::DjamoCi | PaydunyaOperator::DjamoSn => {
                Self::Djamo(PaydunyaDjamoRequest {
                    djamo_full_name: common.full_name,
                    djamo_email: common.email,
                    djamo_phone: common.phone_number,
                    code_country: operator.djamo_code_country().unwrap_or("ci"),
                    djamo_payment_token: common.payment_token,
                })
            }
            PaydunyaOperator::TmoneyTogo => Self::TmoneyTogo(PaydunyaTmoneyTogoRequest {
                name_t_money: common.full_name,
                email_t_money: common.email,
                phone_t_money: common.phone_number,
                payment_token: common.payment_token,
            }),
            PaydunyaOperator::WizallSenegal => Self::WizallSenegal(PaydunyaWizallSenegalRequest {
                wizall_senegal_full_name: common.full_name,
                wizall_senegal_email: common.email,
                wizall_senegal_phone: common.phone_number,
                wizall_senegal_payment_token: common.payment_token,
            }),
        };

        Ok(request)
    }
}

#[derive(Debug)]
pub struct PaydunyaAuthType {
    pub(super) master_key: Secret<String>,
    pub(super) private_key: Secret<String>,
    pub(super) token: Secret<String>,
}

impl TryFrom<&ConnectorAuthType> for PaydunyaAuthType {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(auth_type: &ConnectorAuthType) -> Result<Self, Self::Error> {
        match auth_type {
            ConnectorAuthType::SignatureKey {
                api_key,
                api_secret,
                key1,
            } => Ok(Self {
                master_key: api_key.to_owned(),
                private_key: api_secret.to_owned(),
                token: key1.to_owned(),
            }),
            _ => Err(errors::ConnectorError::FailedToObtainAuthType.into()),
        }
    }
}
// PaymentsResponse
//
// SOFTPAY rails (e.g. MTN Benin) respond to the authorize call with a simple
// envelope rather than a transaction object:
//
//   { "success": true, "message": "...en cours de traitement...", "fees": 100, "currency": "XOF" }
//
// `success: true` only means the debit was *initiated*; the customer still has
// to validate it on their handset (SMS/USSD). The final outcome arrives later
// via the IPN webhook / PSync (`checkout-invoice/confirm/{token}`), so we treat
// a successful initiation as `Pending`. Unknown extra fields (`fees`,
// `currency`, ...) are ignored by serde.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaydunyaPaymentsResponse {
    pub success: bool,
    #[serde(default)]
    pub message: Option<String>,
}

impl
    TryFrom<
        ResponseRouterData<
            Authorize,
            PaydunyaPaymentsResponse,
            PaymentsAuthorizeData,
            PaymentsResponseData,
        >,
    > for RouterData<Authorize, PaymentsAuthorizeData, PaymentsResponseData>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: ResponseRouterData<
            Authorize,
            PaydunyaPaymentsResponse,
            PaymentsAuthorizeData,
            PaymentsResponseData,
        >,
    ) -> Result<Self, Self::Error> {
        let invoice_token = item.data.request.order_id.clone();

        if item.response.success {
            let resource_id = invoice_token
                .clone()
                .map(ResponseId::ConnectorTransactionId)
                .unwrap_or(ResponseId::NoResponseId);
            Ok(Self {
                status: common_enums::AttemptStatus::Pending,
                response: Ok(PaymentsResponseData::TransactionResponse {
                    resource_id,
                    redirection_data: Box::new(None),
                    mandate_reference: Box::new(None),
                    connector_metadata: None,
                    network_txn_id: None,
                    connector_response_reference_id: invoice_token,
                    incremental_authorization_allowed: None,
                    authentication_data: None,
                    charges: None,
                }),
                ..item.data
            })
        } else {
            Ok(Self {
                status: common_enums::AttemptStatus::Failure,
                response: Err(ErrorResponse {
                    status_code: item.http_code,
                    code: NO_ERROR_CODE.to_string(),
                    message: item
                        .response
                        .message
                        .clone()
                        .unwrap_or_else(|| NO_ERROR_MESSAGE.to_string()),
                    reason: item.response.message.clone(),
                    attempt_status: None,
                    connector_transaction_id: invoice_token,
                    connector_response_reference_id: None,
                    network_advice_code: None,
                    network_decline_code: None,
                    network_error_message: None,
                    connector_metadata: None,
                }),
                ..item.data
            })
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PaydunyaSyncStatus {
    #[default]
    Pending,
    Completed,
    Cancelled,
    Failed,
}

impl From<PaydunyaSyncStatus> for common_enums::AttemptStatus {
    fn from(item: PaydunyaSyncStatus) -> Self {
        match item {
            PaydunyaSyncStatus::Completed => Self::Charged,
            PaydunyaSyncStatus::Pending => Self::Pending,
            // Paydunya treats `cancelled` as a terminal non-success state
            // (either user-cancelled or auto-cancelled by inactivity), so we
            // surface it as a failure to upstream.
            PaydunyaSyncStatus::Cancelled | PaydunyaSyncStatus::Failed => Self::Failure,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaydunyaSyncInvoice {
    pub token: String,
    #[serde(default)]
    pub total_amount: Option<serde_json::Value>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaydunyaSyncErrors {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaydunyaSyncResponse {
    pub response_code: String,
    // Paydunya does not always populate `response_text` (it is absent on some
    // statuses, and the IPN body — which is replayed through this struct on the
    // source-verified webhook path — carries it as optional). Keeping it
    // required caused a deserialization failure that surfaced as a webhook
    // processing error, so it must be optional.
    #[serde(default)]
    pub response_text: Option<String>,
    #[serde(default)]
    pub hash: Option<Secret<String>>,
    pub status: PaydunyaSyncStatus,
    #[serde(default)]
    pub fail_reason: Option<String>,
    #[serde(default)]
    pub invoice: Option<PaydunyaSyncInvoice>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub receipt_url: Option<String>,
    #[serde(default)]
    pub customer: Option<serde_json::Value>,
    #[serde(default)]
    pub errors: Option<PaydunyaSyncErrors>,
}

impl PaydunyaSyncResponse {
    fn failure_reason(&self) -> Option<String> {
        self.errors
            .as_ref()
            .and_then(|e| e.description.clone().or_else(|| e.message.clone()))
            .or_else(|| {
                self.fail_reason
                    .clone()
                    .filter(|reason| !reason.trim().is_empty())
            })
    }
}

impl
    TryFrom<ResponseRouterData<PSync, PaydunyaSyncResponse, PaymentsSyncData, PaymentsResponseData>>
    for RouterData<PSync, PaymentsSyncData, PaymentsResponseData>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: ResponseRouterData<
            PSync,
            PaydunyaSyncResponse,
            PaymentsSyncData,
            PaymentsResponseData,
        >,
    ) -> Result<Self, Self::Error> {
        let attempt_status = common_enums::AttemptStatus::from(item.response.status);

        // Paydunya keys every PSync on the invoice token, so we prefer the
        // token echoed back inside the `invoice` block and fall back to the
        // id we originally sent on the request — that way the upstream
        // RouterData keeps a stable connector_transaction_id even when the
        // payload omits the invoice.
        let resource_id = item
            .response
            .invoice
            .as_ref()
            .map(|inv| inv.token.clone())
            .or_else(|| match &item.data.request.connector_transaction_id {
                ResponseId::ConnectorTransactionId(id) => Some(id.clone()),
                _ => None,
            })
            .map(ResponseId::ConnectorTransactionId)
            .unwrap_or(ResponseId::NoResponseId);

        let response = if matches!(attempt_status, common_enums::AttemptStatus::Failure) {
            let reason = item.response.failure_reason();
            Err(ErrorResponse {
                status_code: item.http_code,
                // Paydunya only returns a transport-level `response_code` (e.g.
                // "00" for "Transaction Found"); the rail's actual decline code
                // is not exposed, so we surface our generic placeholder.
                code: NO_ERROR_CODE.to_string(),
                message: reason
                    .clone()
                    .unwrap_or_else(|| NO_ERROR_MESSAGE.to_string()),
                reason,
                attempt_status: Some(attempt_status),
                connector_transaction_id: match &resource_id {
                    ResponseId::ConnectorTransactionId(id) => Some(id.clone()),
                    _ => None,
                },
                connector_response_reference_id: None,
                network_advice_code: None,
                network_decline_code: None,
                network_error_message: None,
                connector_metadata: None,
            })
        } else {
            Ok(PaymentsResponseData::TransactionResponse {
                resource_id,
                redirection_data: Box::new(None),
                mandate_reference: Box::new(None),
                connector_metadata: None,
                network_txn_id: None,
                connector_response_reference_id: None,
                incremental_authorization_allowed: None,
                authentication_data: None,
                charges: None,
            })
        };

        Ok(Self {
            status: attempt_status,
            response,
            ..item.data
        })
    }
}

//TODO: Fill the struct with respective fields
// REFUND :
// Type definition for RefundRequest
#[derive(Default, Debug, Serialize)]
pub struct PaydunyaRefundRequest {
    pub amount: MinorUnit,
}

impl<F> TryFrom<&PaydunyaRouterData<&RefundsRouterData<F>>> for PaydunyaRefundRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(item: &PaydunyaRouterData<&RefundsRouterData<F>>) -> Result<Self, Self::Error> {
        Ok(Self {
            amount: item.amount.to_owned(),
        })
    }
}

// Type definition for Refund Response

#[allow(dead_code)]
#[derive(Debug, Copy, Serialize, Default, Deserialize, Clone)]
pub enum RefundStatus {
    Succeeded,
    Failed,
    #[default]
    Processing,
}

impl From<RefundStatus> for enums::RefundStatus {
    fn from(item: RefundStatus) -> Self {
        match item {
            RefundStatus::Succeeded => Self::Success,
            RefundStatus::Failed => Self::Failure,
            RefundStatus::Processing => Self::Pending,
            //TODO: Review mapping
        }
    }
}

//TODO: Fill the struct with respective fields
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct RefundResponse {
    id: String,
    status: RefundStatus,
}

impl TryFrom<RefundsResponseRouterData<Execute, RefundResponse>> for RefundsRouterData<Execute> {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: RefundsResponseRouterData<Execute, RefundResponse>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            response: Ok(RefundsResponseData {
                connector_refund_id: item.response.id.to_string(),
                refund_status: enums::RefundStatus::from(item.response.status),
            }),
            ..item.data
        })
    }
}

impl TryFrom<RefundsResponseRouterData<RSync, RefundResponse>> for RefundsRouterData<RSync> {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: RefundsResponseRouterData<RSync, RefundResponse>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            response: Ok(RefundsResponseData {
                connector_refund_id: item.response.id.to_string(),
                refund_status: enums::RefundStatus::from(item.response.status),
            }),
            ..item.data
        })
    }
}

// Paydunya error bodies are JSON objects that always carry at least the
// `response_code` and `response_text` nodes (see Paydunya HTTP/JSON docs).
// Every field is optional so a JSON body never fails to deserialize; the
// connector falls back to the raw body when the response isn't JSON at all
// (e.g. an HTML 404/5xx page).
#[derive(Default, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaydunyaErrorResponse {
    #[serde(default)]
    pub response_code: Option<String>,
    #[serde(default)]
    pub response_text: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

// =====================================================================
// IPN (Instant Payment Notification)
// =====================================================================
//
// Paydunya posts IPN payloads to the `callback_url` declared on the invoice.
// The body is `application/x-www-form-urlencoded`, with PHP-style nested keys
// scoped under a top-level `data` key:
//
//   data[response_code]=00
//   data[response_text]=Transaction+Found
//   data[hash]=<sha512(master_key)>
//   data[status]=completed
//   data[invoice][token]=test_jkEdPY8SuG
//   data[invoice][total_amount]=42300
//   ...
//
// The `hash` field is `SHA-512(MasterKey)` (hex-encoded) — Paydunya's docs
// describe this as the only piece of authenticity data on the IPN.
//
// We deserialize via `serde_qs`, which handles the bracket notation natively.
// Fields we don't currently consume (customer, items, taxes, ...) are kept as
// `serde_json::Value` so future flows can grow into them without forcing a
// strict schema today.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaydunyaIpnEnvelope {
    pub data: PaydunyaIpnBody,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaydunyaIpnInvoice {
    pub token: String,
    #[serde(default)]
    pub total_amount: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaydunyaIpnBody {
    pub response_code: String,
    #[serde(default)]
    pub response_text: Option<String>,
    pub hash: Secret<String>,
    pub status: PaydunyaSyncStatus,
    pub invoice: PaydunyaIpnInvoice,
    #[serde(default)]
    pub fail_reason: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub receipt_url: Option<String>,
    #[serde(default)]
    pub customer: Option<serde_json::Value>,
}

impl PaydunyaIpnBody {
    pub fn invoice_token(&self) -> &str {
        &self.invoice.token
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use common_enums::{AttemptStatus, RefundStatus as CommonRefundStatus};
    use hyperswitch_masking::{PeekInterface, Secret};

    use super::*;

    // ---------------------------------------------------------------
    // PaydunyaOperator
    // ---------------------------------------------------------------

    #[test]
    fn operator_endpoint_matches_softpay_paths() {
        // Endpoints are part of the connector contract — a mistyped path
        // here means Paydunya rejects the call with a 404, so pin every
        // variant explicitly.
        assert_eq!(PaydunyaOperator::MtnBenin.endpoint(), "softpay/mtn-benin");
        assert_eq!(PaydunyaOperator::MtnCi.endpoint(), "softpay/mtn-ci");
        assert_eq!(
            PaydunyaOperator::MtnCameroun.endpoint(),
            "softpay/mtn-cameroun"
        );
        assert_eq!(PaydunyaOperator::MoovBenin.endpoint(), "softpay/moov-benin");
        assert_eq!(PaydunyaOperator::MoovCi.endpoint(), "softpay/moov-ci");
        assert_eq!(PaydunyaOperator::MoovMali.endpoint(), "softpay/moov-mali");
        assert_eq!(PaydunyaOperator::MoovTogo.endpoint(), "softpay/moov-togo");
        assert_eq!(
            PaydunyaOperator::MoovBurkina.endpoint(),
            "softpay/moov-burkina"
        );
        assert_eq!(
            PaydunyaOperator::OrangeMoneyCi.endpoint(),
            "softpay/orange-money-ci"
        );
        assert_eq!(
            PaydunyaOperator::OrangeMoneySenegal.endpoint(),
            "softpay/new-orange-money-senegal"
        );
        assert_eq!(
            PaydunyaOperator::OrangeMoneyMali.endpoint(),
            "softpay/orange-money-mali"
        );
        assert_eq!(
            PaydunyaOperator::OrangeMoneyBurkina.endpoint(),
            "softpay/orange-money-burkina"
        );
        assert_eq!(
            PaydunyaOperator::WaveSenegal.endpoint(),
            "softpay/wave-senegal"
        );
        assert_eq!(PaydunyaOperator::WaveCi.endpoint(), "softpay/wave-ci");
        assert_eq!(
            PaydunyaOperator::FreeMoneySenegal.endpoint(),
            "softpay/free-money-senegal"
        );
        assert_eq!(
            PaydunyaOperator::ExpressoSenegal.endpoint(),
            "softpay/expresso-senegal"
        );
        // Côte d'Ivoire and Senegal deliberately share one endpoint.
        assert_eq!(PaydunyaOperator::DjamoCi.endpoint(), "softpay/djamo");
        assert_eq!(PaydunyaOperator::DjamoSn.endpoint(), "softpay/djamo");
        assert_eq!(
            PaydunyaOperator::TmoneyTogo.endpoint(),
            "softpay/t-money-togo"
        );
        assert_eq!(
            PaydunyaOperator::WizallSenegal.endpoint(),
            "softpay/wizall-senegal"
        );
    }

    #[test]
    fn djamo_code_country_is_set_only_for_djamo_family() {
        assert_eq!(PaydunyaOperator::DjamoCi.djamo_code_country(), Some("ci"));
        assert_eq!(PaydunyaOperator::DjamoSn.djamo_code_country(), Some("sn"));
        assert_eq!(PaydunyaOperator::WaveCi.djamo_code_country(), None);
        assert_eq!(PaydunyaOperator::MtnBenin.djamo_code_country(), None);
        assert_eq!(PaydunyaOperator::TmoneyTogo.djamo_code_country(), None);
    }

    #[test]
    fn wallet_provider_is_set_only_for_mtn_family() {
        // Only the MTN SOFTPAY endpoints require the `*_wallet_provider`
        // discriminator — every other operator must omit it (returns None).
        assert_eq!(
            PaydunyaOperator::MtnBenin.wallet_provider(),
            Some("MTNBENIN")
        );
        assert_eq!(PaydunyaOperator::MtnCi.wallet_provider(), Some("MTNCI"));
        assert_eq!(
            PaydunyaOperator::MtnCameroun.wallet_provider(),
            Some("MTNCAMEROUN")
        );

        for non_mtn in [
            PaydunyaOperator::MoovBenin,
            PaydunyaOperator::MoovCi,
            PaydunyaOperator::MoovMali,
            PaydunyaOperator::MoovTogo,
            PaydunyaOperator::MoovBurkina,
            PaydunyaOperator::OrangeMoneyCi,
            PaydunyaOperator::OrangeMoneySenegal,
            PaydunyaOperator::OrangeMoneyMali,
            PaydunyaOperator::OrangeMoneyBurkina,
            PaydunyaOperator::WaveSenegal,
            PaydunyaOperator::WaveCi,
            PaydunyaOperator::FreeMoneySenegal,
            PaydunyaOperator::ExpressoSenegal,
            PaydunyaOperator::DjamoCi,
            PaydunyaOperator::DjamoSn,
            PaydunyaOperator::TmoneyTogo,
            PaydunyaOperator::WizallSenegal,
        ] {
            assert_eq!(non_mtn.wallet_provider(), None);
        }
    }

    // ---------------------------------------------------------------
    // Status conversions
    // ---------------------------------------------------------------

    #[test]
    fn softpay_response_deserializes_success_envelope() {
        // MTN Benin SOFTPAY success envelope: initiation accepted, awaiting
        // customer confirmation. Extra fields (`fees`, `currency`) are ignored.
        let body = r#"{"success": true, "message": "Votre paiement est en cours de traitement.", "fees": 100, "currency": "XOF"}"#;
        let parsed: PaydunyaPaymentsResponse = serde_json::from_str(body).unwrap();
        assert!(parsed.success);
        assert_eq!(
            parsed.message.as_deref(),
            Some("Votre paiement est en cours de traitement.")
        );
    }

    #[test]
    fn softpay_response_deserializes_failure_envelope() {
        let body = r#"{"success": false, "message": "Le compte n'existe pas"}"#;
        let parsed: PaydunyaPaymentsResponse = serde_json::from_str(body).unwrap();
        assert!(!parsed.success);
        assert_eq!(parsed.message.as_deref(), Some("Le compte n'existe pas"));
    }

    #[test]
    fn sync_status_collapses_cancelled_into_failure() {
        // Paydunya auto-cancels invoices after 24h, but upstream wants a
        // terminal failure — the From impl must treat Cancelled and Failed
        // identically.
        assert_eq!(
            AttemptStatus::from(PaydunyaSyncStatus::Completed),
            AttemptStatus::Charged
        );
        assert_eq!(
            AttemptStatus::from(PaydunyaSyncStatus::Pending),
            AttemptStatus::Pending
        );
        assert_eq!(
            AttemptStatus::from(PaydunyaSyncStatus::Cancelled),
            AttemptStatus::Failure
        );
        assert_eq!(
            AttemptStatus::from(PaydunyaSyncStatus::Failed),
            AttemptStatus::Failure
        );
    }

    #[test]
    fn refund_status_maps_to_common_refund_status() {
        assert_eq!(
            CommonRefundStatus::from(RefundStatus::Succeeded),
            CommonRefundStatus::Success
        );
        assert_eq!(
            CommonRefundStatus::from(RefundStatus::Failed),
            CommonRefundStatus::Failure
        );
        assert_eq!(
            CommonRefundStatus::from(RefundStatus::Processing),
            CommonRefundStatus::Pending
        );
    }

    // ---------------------------------------------------------------
    // PaydunyaAuthType
    // ---------------------------------------------------------------

    #[test]
    fn auth_type_extracts_all_three_keys_from_signature_key() {
        // Mapping mirrors `connector_configs/toml/*.toml`:
        //   api_key    -> Master Key
        //   api_secret -> Private Key
        //   key1       -> Token
        let auth = ConnectorAuthType::SignatureKey {
            api_key: Secret::new("master-key".to_string()),
            api_secret: Secret::new("private-key".to_string()),
            key1: Secret::new("token".to_string()),
        };

        let parsed = PaydunyaAuthType::try_from(&auth).unwrap();
        assert_eq!(parsed.master_key.peek(), "master-key");
        assert_eq!(parsed.private_key.peek(), "private-key");
        assert_eq!(parsed.token.peek(), "token");
    }

    #[test]
    fn auth_type_rejects_non_signature_variants() {
        // Any auth shape other than `SignatureKey` should bubble up as a
        // `FailedToObtainAuthType` so the framework can return a clean 4xx.
        let auth = ConnectorAuthType::HeaderKey {
            api_key: Secret::new("nope".to_string()),
        };
        let err = PaydunyaAuthType::try_from(&auth).unwrap_err();
        assert!(matches!(
            err.current_context(),
            errors::ConnectorError::FailedToObtainAuthType
        ));
    }

    // ---------------------------------------------------------------
    // PaydunyaSyncResponse::failure_reason
    // ---------------------------------------------------------------

    fn sync_response_with(
        fail_reason: Option<&str>,
        errors_block: Option<PaydunyaSyncErrors>,
    ) -> PaydunyaSyncResponse {
        PaydunyaSyncResponse {
            response_code: "00".to_string(),
            response_text: Some("Transaction Found".to_string()),
            hash: None,
            status: PaydunyaSyncStatus::Failed,
            fail_reason: fail_reason.map(str::to_string),
            invoice: None,
            mode: None,
            receipt_url: None,
            customer: None,
            errors: errors_block,
        }
    }

    #[test]
    fn failure_reason_prefers_errors_description() {
        let response = sync_response_with(
            Some("top-level"),
            Some(PaydunyaSyncErrors {
                message: Some("err-message".to_string()),
                description: Some("err-description".to_string()),
            }),
        );
        assert_eq!(
            response.failure_reason().as_deref(),
            Some("err-description")
        );
    }

    #[test]
    fn failure_reason_falls_back_to_errors_message() {
        let response = sync_response_with(
            Some("top-level"),
            Some(PaydunyaSyncErrors {
                message: Some("err-message".to_string()),
                description: None,
            }),
        );
        assert_eq!(response.failure_reason().as_deref(), Some("err-message"));
    }

    #[test]
    fn failure_reason_falls_back_to_fail_reason_when_errors_empty() {
        let response = sync_response_with(
            Some("softpay-decline"),
            Some(PaydunyaSyncErrors {
                message: None,
                description: None,
            }),
        );
        assert_eq!(
            response.failure_reason().as_deref(),
            Some("softpay-decline")
        );
    }

    #[test]
    fn failure_reason_uses_fail_reason_when_errors_missing() {
        let response = sync_response_with(Some("softpay-decline"), None);
        assert_eq!(
            response.failure_reason().as_deref(),
            Some("softpay-decline")
        );
    }

    #[test]
    fn failure_reason_ignores_blank_fail_reason() {
        // Paydunya sometimes returns an empty string instead of dropping the
        // field — treating that as a "real" reason would put whitespace into
        // the merchant-facing error message, so filter it out.
        let response = sync_response_with(Some("   "), None);
        assert!(response.failure_reason().is_none());
    }

    #[test]
    fn failure_reason_returns_none_when_no_signal() {
        let response = sync_response_with(None, None);
        assert!(response.failure_reason().is_none());
    }

    // ---------------------------------------------------------------
    // PaydunyaSyncResponse JSON deserialization
    // ---------------------------------------------------------------

    #[test]
    fn sync_response_deserializes_full_payload() {
        // Shape mirrors the official `checkout-invoice/confirm/{token}` doc.
        let body = r#"{
            "response_code": "00",
            "response_text": "Transaction Found",
            "hash": "abcdef",
            "status": "completed",
            "invoice": {
                "token": "test_jkEdPY8SuG",
                "total_amount": 42300,
                "description": "test invoice"
            },
            "mode": "test",
            "receipt_url": "https://paydunya.com/receipt/test_jkEdPY8SuG",
            "customer": {"name": "John", "email": "john@example.com"}
        }"#;

        let response: PaydunyaSyncResponse = serde_json::from_str(body).unwrap();
        assert_eq!(response.response_code, "00");
        assert_eq!(response.status, PaydunyaSyncStatus::Completed);
        assert_eq!(response.hash.as_ref().unwrap().peek(), "abcdef");
        let invoice = response.invoice.as_ref().unwrap();
        assert_eq!(invoice.token, "test_jkEdPY8SuG");
        assert_eq!(invoice.description.as_deref(), Some("test invoice"));
    }

    #[test]
    fn sync_response_accepts_missing_optional_fields() {
        // The minimal payload Paydunya sends back when an invoice token is
        // unknown — no invoice block, no hash, no errors. Deserialization
        // must still succeed so we can surface a generic "not found" error.
        let body = r#"{
            "response_code": "404",
            "response_text": "Invoice introuvable",
            "status": "failed"
        }"#;

        let response: PaydunyaSyncResponse = serde_json::from_str(body).unwrap();
        assert_eq!(response.status, PaydunyaSyncStatus::Failed);
        assert!(response.invoice.is_none());
        assert!(response.hash.is_none());
        assert!(response.errors.is_none());
    }

    // ---------------------------------------------------------------
    // PaydunyaIpnEnvelope (urlencoded webhook body)
    // ---------------------------------------------------------------

    // Matches the runtime config used by `parse_paydunya_ipn`: non-strict so
    // percent-encoded brackets (`%5B`/`%5D`) are decoded, with extra depth for
    // Paydunya's nested `invoice.items[...]` blocks.
    fn parse_ipn(body: &str) -> PaydunyaIpnEnvelope {
        serde_qs::Config::new(10, false)
            .deserialize_bytes(body.as_bytes())
            .unwrap()
    }

    #[test]
    fn ipn_envelope_decodes_bracket_form_body() {
        // Paydunya posts IPNs as `application/x-www-form-urlencoded` with
        // PHP-style nested keys.
        let body = "data[response_code]=00\
                    &data[response_text]=Transaction+Found\
                    &data[hash]=deadbeef\
                    &data[status]=completed\
                    &data[invoice][token]=test_jkEdPY8SuG\
                    &data[invoice][total_amount]=42300\
                    &data[invoice][description]=test+invoice\
                    &data[mode]=test\
                    &data[receipt_url]=https://paydunya.com/receipt/test_jkEdPY8SuG";

        let ipn = parse_ipn(body).data;

        assert_eq!(ipn.response_code, "00");
        assert_eq!(ipn.response_text.as_deref(), Some("Transaction Found"));
        assert_eq!(ipn.status, PaydunyaSyncStatus::Completed);
        assert_eq!(ipn.hash.peek(), "deadbeef");
        assert_eq!(ipn.invoice.token, "test_jkEdPY8SuG");
        // Paydunya serialises integers as strings inside the form body, so
        // `total_amount` is kept as Option<String>.
        assert_eq!(ipn.invoice.total_amount.as_deref(), Some("42300"));
        assert_eq!(ipn.invoice_token(), "test_jkEdPY8SuG");
    }

    #[test]
    fn ipn_envelope_decodes_percent_encoded_bracket_body() {
        // The real Paydunya IPN is sent by a PHP/Guzzle client, which
        // percent-encodes the bracket keys (`data%5Binvoice%5D%5Btoken%5D`).
        // serde_qs' default strict parser does NOT decode `%5B`/`%5D` and fails
        // with "missing field `data`" (observed as a `WE_03` webhook error), so
        // this must parse via the non-strict runtime config. Includes the
        // nested `items`/`taxes`/`custom_data` blocks real IPNs carry.
        let body = "data%5Bresponse_code%5D=00\
                    &data%5Bresponse_text%5D=Transaction+Found\
                    &data%5Bhash%5D=deadbeef\
                    &data%5Bstatus%5D=completed\
                    &data%5Binvoice%5D%5Btoken%5D=test_jkEdPY8SuG\
                    &data%5Binvoice%5D%5Btotal_amount%5D=42300\
                    &data%5Binvoice%5D%5Bitems%5D%5Bitem_0%5D%5Bname%5D=Product\
                    &data%5Binvoice%5D%5Btaxes%5D%5Btax_0%5D%5Bname%5D=TVA\
                    &data%5Bcustom_data%5D%5Border_id%5D=abc123\
                    &data%5Bmode%5D=live";

        let ipn = parse_ipn(body).data;

        assert_eq!(ipn.response_code, "00");
        assert_eq!(ipn.status, PaydunyaSyncStatus::Completed);
        assert_eq!(ipn.hash.peek(), "deadbeef");
        assert_eq!(ipn.invoice.token, "test_jkEdPY8SuG");
        assert_eq!(ipn.invoice_token(), "test_jkEdPY8SuG");
    }

    #[test]
    fn ipn_envelope_decodes_cancelled_status() {
        // Auto-cancellation by inactivity uses the same opcode as a manual
        // cancel, so we still expect `cancelled` to flow through cleanly.
        let body = "data[response_code]=00\
                    &data[hash]=deadbeef\
                    &data[status]=cancelled\
                    &data[invoice][token]=cancelled_token\
                    &data[fail_reason]=customer_cancelled";

        let ipn = parse_ipn(body).data;

        assert_eq!(ipn.status, PaydunyaSyncStatus::Cancelled);
        assert_eq!(ipn.invoice_token(), "cancelled_token");
        assert_eq!(ipn.fail_reason.as_deref(), Some("customer_cancelled"));
    }

    // ---------------------------------------------------------------
    // PaydunyaPaymentsRequest serialization
    // ---------------------------------------------------------------

    fn email(addr: &str) -> Email {
        Email::from_str(addr).unwrap()
    }

    #[test]
    fn mtn_benin_request_serializes_with_mtn_prefixed_fields() {
        let req = PaydunyaPaymentsRequest::MtnBenin(PaydunyaMtnBeninRequest {
            mtn_benin_customer_fullname: Secret::new("John Doe".to_string()),
            mtn_benin_email: email("john@example.com"),
            mtn_benin_phone_number: Secret::new("22990000000".to_string()),
            mtn_benin_wallet_provider: "MTNBENIN",
            payment_token: "tok_123".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        // `untagged` enum means the variant body is inlined at the top level.
        assert_eq!(value["mtn_benin_customer_fullname"], "John Doe");
        assert_eq!(value["mtn_benin_email"], "john@example.com");
        assert_eq!(value["mtn_benin_phone_number"], "22990000000");
        assert_eq!(value["mtn_benin_wallet_provider"], "MTNBENIN");
        assert_eq!(value["payment_token"], "tok_123");
    }

    #[test]
    fn wave_senegal_request_renames_full_name_to_camel_case() {
        // Paydunya's Wave SOFTPAY endpoint expects `wave_senegal_fullName`
        // (camelCase). A rename regression here would silently drop the
        // payer's name on the connector side.
        let req = PaydunyaPaymentsRequest::WaveSenegal(PaydunyaWaveSenegalRequest {
            wave_senegal_full_name: Secret::new("Awa Ndiaye".to_string()),
            wave_senegal_email: email("awa@example.com"),
            wave_senegal_phone: Secret::new("221770000000".to_string()),
            wave_senegal_payment_token: "tok_wave".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert!(value.get("wave_senegal_full_name").is_none());
        assert_eq!(value["wave_senegal_fullName"], "Awa Ndiaye");
        assert_eq!(value["wave_senegal_email"], "awa@example.com");
        assert_eq!(value["wave_senegal_phone"], "221770000000");
        assert_eq!(value["wave_senegal_payment_token"], "tok_wave");
    }

    #[test]
    fn wizall_senegal_request_renames_full_name_to_camel_case() {
        // Wizall Senegal's SOFTPAY endpoint expects `wizall_senegal_fullName`
        // (camelCase) plus an operator-prefixed `wizall_senegal_payment_token`,
        // mirroring the Wave Senegal layout. A rename regression here would
        // silently drop the payer's name or token on the connector side.
        let req = PaydunyaPaymentsRequest::WizallSenegal(PaydunyaWizallSenegalRequest {
            wizall_senegal_full_name: Secret::new("Awa Ndiaye".to_string()),
            wizall_senegal_email: email("awa@example.com"),
            wizall_senegal_phone: Secret::new("221770000000".to_string()),
            wizall_senegal_payment_token: "tok_wizall".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert!(value.get("wizall_senegal_full_name").is_none());
        assert_eq!(value["wizall_senegal_fullName"], "Awa Ndiaye");
        assert_eq!(value["wizall_senegal_email"], "awa@example.com");
        assert_eq!(value["wizall_senegal_phone"], "221770000000");
        assert_eq!(value["wizall_senegal_payment_token"], "tok_wizall");
        assert!(value.get("payment_token").is_none());
    }

    #[test]
    fn expresso_senegal_request_renames_full_name_to_camel_case() {
        let req = PaydunyaPaymentsRequest::ExpressoSenegal(PaydunyaExpressoSenegalRequest {
            expresso_sn_full_name: Secret::new("Awa Ndiaye".to_string()),
            expresso_sn_email: email("awa@example.com"),
            expresso_sn_phone: Secret::new("221770000000".to_string()),
            payment_token: "tok_expresso".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert!(value.get("expresso_sn_full_name").is_none());
        assert_eq!(value["expresso_sn_fullName"], "Awa Ndiaye");
        assert_eq!(value["payment_token"], "tok_expresso");
    }

    #[test]
    fn djamo_request_renames_full_name_and_carries_code_country() {
        // Djamo's SOFTPAY endpoint expects `djamo_fullName` (camelCase), an
        // operator-prefixed `djamo_payment_token`, and a `code_country`
        // discriminator that routes the shared endpoint to the right region.
        let req = PaydunyaPaymentsRequest::Djamo(PaydunyaDjamoRequest {
            djamo_full_name: Secret::new("Camille Coulibaly".to_string()),
            djamo_email: email("camille@example.com"),
            djamo_phone: Secret::new("0777568646".to_string()),
            code_country: "ci",
            djamo_payment_token: "tok_djamo".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert!(value.get("djamo_full_name").is_none());
        assert_eq!(value["djamo_fullName"], "Camille Coulibaly");
        assert_eq!(value["djamo_email"], "camille@example.com");
        assert_eq!(value["djamo_phone"], "0777568646");
        assert_eq!(value["code_country"], "ci");
        assert_eq!(value["djamo_payment_token"], "tok_djamo");
        assert!(value.get("payment_token").is_none());
    }

    #[test]
    fn tmoney_togo_request_serializes_with_t_money_prefixed_fields() {
        // Togo's T-Money SOFTPAY endpoint uses the short `*_t_money` field
        // names plus the generic `payment_token`. Pin the layout so a rename
        // regression can't silently drop fields against Paydunya.
        let req = PaydunyaPaymentsRequest::TmoneyTogo(PaydunyaTmoneyTogoRequest {
            name_t_money: Secret::new("Camille".to_string()),
            email_t_money: email("camille@example.com"),
            phone_t_money: Secret::new("70707070".to_string()),
            payment_token: "tok_tmoney".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["name_t_money"], "Camille");
        assert_eq!(value["email_t_money"], "camille@example.com");
        assert_eq!(value["phone_t_money"], "70707070");
        assert_eq!(value["payment_token"], "tok_tmoney");
    }

    #[test]
    fn orange_money_senegal_request_uses_generic_field_names() {
        // Unlike MTN/Wave, the Orange Money Senegal endpoint switched to
        // generic field names (`customer_name`, `customer_email`,
        // `phone_number`, `invoice_token`) — pin this so the connector
        // never silently regresses to operator-prefixed names.
        let req = PaydunyaPaymentsRequest::OrangeMoneySenegal(PaydunyaOrangeMoneySenegalRequest {
            customer_name: Secret::new("Awa Ndiaye".to_string()),
            customer_email: email("awa@example.com"),
            phone_number: Secret::new("221770000000".to_string()),
            invoice_token: "tok_om".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["customer_name"], "Awa Ndiaye");
        assert_eq!(value["customer_email"], "awa@example.com");
        assert_eq!(value["phone_number"], "221770000000");
        assert_eq!(value["invoice_token"], "tok_om");
        assert!(value.get("payment_token").is_none());
    }

    #[test]
    fn orange_money_mali_request_serializes_with_mali_prefixed_fields_and_address() {
        // Mali's SOFTPAY endpoint uses operator-prefixed field names AND
        // requires `orange_money_mali_customer_address` (Paydunya's example
        // sends the city). Regressions on either the rename or the address
        // field would silently break authorization.
        let req = PaydunyaPaymentsRequest::OrangeMoneyMali(PaydunyaOrangeMoneyMaliRequest {
            orange_money_mali_customer_fullname: Secret::new("Camille Coulibaly".to_string()),
            orange_money_mali_email: email("camille@example.com"),
            orange_money_mali_phone_number: Secret::new("22390239415".to_string()),
            orange_money_mali_customer_address: Secret::new("Bamako".to_string()),
            payment_token: "tok_mali".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(
            value["orange_money_mali_customer_fullname"],
            "Camille Coulibaly"
        );
        assert_eq!(value["orange_money_mali_email"], "camille@example.com");
        assert_eq!(value["orange_money_mali_phone_number"], "22390239415");
        assert_eq!(value["orange_money_mali_customer_address"], "Bamako");
        assert_eq!(value["payment_token"], "tok_mali");
    }

    #[test]
    fn orange_money_burkina_request_uses_bf_suffixed_fields_with_otp() {
        // Burkina is the odd one out: short `name_bf`/`email_bf`/`phone_bf`
        // field names plus a mandatory `otp_code`. A rename regression here
        // would cause Paydunya to reject the call with a 4xx because the
        // operator-specific keys are missing.
        let req = PaydunyaPaymentsRequest::OrangeMoneyBurkina(PaydunyaOrangeMoneyBurkinaRequest {
            name_bf: Secret::new("Fallou Sawadogo".to_string()),
            email_bf: email("fallou@example.com"),
            phone_bf: Secret::new("22676950976".to_string()),
            otp_code: Secret::new("89525".to_string()),
            payment_token: "tok_bf".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["name_bf"], "Fallou Sawadogo");
        assert_eq!(value["email_bf"], "fallou@example.com");
        assert_eq!(value["phone_bf"], "22676950976");
        assert_eq!(value["otp_code"], "89525");
        assert_eq!(value["payment_token"], "tok_bf");
    }

    #[test]
    fn orange_money_ci_request_serializes_with_ci_prefixed_fields_and_otp() {
        // Côte d'Ivoire uses operator-prefixed field names everywhere PLUS
        // an `orange_money_ci_otp` field the payer generates via USSD
        // `#144*82#`. A rename regression here would silently produce an
        // unauthorized call against Paydunya.
        let req = PaydunyaPaymentsRequest::OrangeMoneyCi(PaydunyaOrangeMoneyCiRequest {
            orange_money_ci_customer_fullname: Secret::new("Adjoa Kouame".to_string()),
            orange_money_ci_email: email("adjoa@example.com"),
            orange_money_ci_phone_number: Secret::new("2250777568646".to_string()),
            orange_money_ci_otp: Secret::new("8562".to_string()),
            payment_token: "tok_ci".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["orange_money_ci_customer_fullname"], "Adjoa Kouame");
        assert_eq!(value["orange_money_ci_email"], "adjoa@example.com");
        assert_eq!(value["orange_money_ci_phone_number"], "2250777568646");
        assert_eq!(value["orange_money_ci_otp"], "8562");
        assert_eq!(value["payment_token"], "tok_ci");
    }

    #[test]
    fn moov_mali_request_serializes_with_ml_prefixed_fields_and_address() {
        let req = PaydunyaPaymentsRequest::MoovMali(PaydunyaMoovMaliRequest {
            moov_ml_customer_fullname: Secret::new("John Doe".to_string()),
            moov_ml_email: email("john@example.com"),
            moov_ml_phone_number: Secret::new("90239415".to_string()),
            moov_ml_customer_address: Secret::new("Bamako".to_string()),
            payment_token: "tok_moov_ml".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["moov_ml_customer_fullname"], "John Doe");
        assert_eq!(value["moov_ml_email"], "john@example.com");
        assert_eq!(value["moov_ml_phone_number"], "90239415");
        assert_eq!(value["moov_ml_customer_address"], "Bamako");
        assert_eq!(value["payment_token"], "tok_moov_ml");
    }

    #[test]
    fn moov_togo_request_serializes_with_togo_prefixed_fields_and_address() {
        let req = PaydunyaPaymentsRequest::MoovTogo(PaydunyaMoovTogoRequest {
            moov_togo_customer_fullname: Secret::new("Kofi Mensah".to_string()),
            moov_togo_email: email("kofi@example.com"),
            moov_togo_customer_address: Secret::new("Lome".to_string()),
            moov_togo_phone_number: Secret::new("12345678".to_string()),
            payment_token: "tok_moov_togo".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["moov_togo_customer_fullname"], "Kofi Mensah");
        assert_eq!(value["moov_togo_email"], "kofi@example.com");
        assert_eq!(value["moov_togo_customer_address"], "Lome");
        assert_eq!(value["moov_togo_phone_number"], "12345678");
        assert_eq!(value["payment_token"], "tok_moov_togo");
    }

    #[test]
    fn moov_burkina_request_uses_camel_case_name_and_prefixed_token() {
        // Burkina's Moov endpoint is the odd one out: the full-name key is
        // camelCased (`moov_burkina_faso_fullName`) and the invoice token
        // rides on an operator-prefixed `moov_burkina_faso_payment_token`
        // rather than the generic `payment_token`. Pin both so a rename
        // regression can't silently drop fields against Paydunya.
        let req = PaydunyaPaymentsRequest::MoovBurkina(PaydunyaMoovBurkinaRequest {
            moov_burkina_faso_full_name: Secret::new("Fallou".to_string()),
            moov_burkina_faso_email: email("fallou@example.com"),
            moov_burkina_faso_phone_number: Secret::new("51765664".to_string()),
            moov_burkina_faso_payment_token: "tok_moov_bf".to_string(),
        });

        let value = serde_json::to_value(&req).unwrap();
        assert!(value.get("moov_burkina_faso_full_name").is_none());
        assert_eq!(value["moov_burkina_faso_fullName"], "Fallou");
        assert_eq!(value["moov_burkina_faso_email"], "fallou@example.com");
        assert_eq!(value["moov_burkina_faso_phone_number"], "51765664");
        assert_eq!(value["moov_burkina_faso_payment_token"], "tok_moov_bf");
        assert!(value.get("payment_token").is_none());
    }

    // ---------------------------------------------------------------
    // extract_orange_money_otp — typed OTP transport
    // ---------------------------------------------------------------

    #[test]
    fn extract_otp_returns_some_when_wallet_carries_orange_money_redirect() {
        // The OTP transport rides on the typed wallet variant so card-rail
        // and non-Orange-Money wallets can't accidentally leak an OTP into
        // the SOFTPAY request.
        let pm_data = PaymentMethodData::Wallet(WalletData::OrangeMoneyRedirect(Box::new(
            hyperswitch_domain_models::payment_method_data::OrangeMoneyRedirection {
                otp: Some(Secret::new("8562".to_string())),
            },
        )));
        let extracted = extract_orange_money_otp(&pm_data).expect("OTP should be extracted");
        assert_eq!(extracted.peek(), "8562");
    }

    #[test]
    fn extract_otp_returns_none_when_orange_money_redirect_otp_missing() {
        // Some Orange Money variants don't need an OTP at all (e.g.
        // Senegal QR, Mali). The helper must return None — not a blank
        // Secret — so the operator-specific arm can reject upfront.
        let pm_data = PaymentMethodData::Wallet(WalletData::OrangeMoneyRedirect(Box::new(
            hyperswitch_domain_models::payment_method_data::OrangeMoneyRedirection { otp: None },
        )));
        assert!(extract_orange_money_otp(&pm_data).is_none());
    }

    #[test]
    fn extract_otp_returns_none_for_non_orange_money_wallets() {
        // Confirm that the typed pattern match is strict — any other wallet
        // variant (e.g. MoMo) returns None so the missing-field error
        // message stays accurate instead of silently leaking an empty OTP.
        let pm_data = PaymentMethodData::Wallet(WalletData::MomoRedirect(
            hyperswitch_domain_models::payment_method_data::MomoRedirection {},
        ));
        assert!(extract_orange_money_otp(&pm_data).is_none());
    }

    // ---------------------------------------------------------------
    // Preprocessing
    // ---------------------------------------------------------------

    #[test]
    fn preprocessing_request_serializes_with_expected_layout() {
        // The Paydunya `checkout-invoice/create` endpoint expects a strict
        // nested shape (`invoice.total_amount`, `store.name`,
        // `actions.callback_url`, `actions.return_url`). The struct is
        // simple enough to test by hand-constructing it.
        let request = PaydunyaPreprocessingRequest {
            invoice: Invoice {
                total_amount: MinorUnit::new(1500),
            },
            store: Store {
                name: "name".to_string(),
            },
            actions: Actions {
                callback_url: "https://example.com/webhook".to_string(),
                return_url: "https://example.com/return".to_string(),
            },
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["invoice"]["total_amount"], 1500);
        assert_eq!(value["store"]["name"], "name");
        assert_eq!(
            value["actions"]["callback_url"],
            "https://example.com/webhook"
        );
        assert_eq!(value["actions"]["return_url"], "https://example.com/return");
    }

    #[test]
    fn metadata_parses_store_name_from_connector_meta_data() {
        // The invoice `store.name` is now sourced from
        // `connector_meta_data.store_name` rather than being hardcoded, so
        // confirm the metadata object pulls it back out intact.
        let meta = common_utils::pii::SecretSerdeValue::new(serde_json::json!({
            "store_name": "My Store"
        }));
        let parsed = PaydunyaConnectorMetadataObject::try_from(Some(&meta))
            .expect("store_name should parse");
        assert_eq!(parsed.store_name, "My Store");
    }

    #[test]
    fn metadata_missing_returns_invalid_connector_config() {
        // Without connector metadata there is no store name to put on the
        // invoice, so the connector must surface InvalidConnectorConfig
        // instead of sending an empty store to Paydunya.
        let err = PaydunyaConnectorMetadataObject::try_from(None)
            .expect_err("missing metadata must error");
        assert!(matches!(
            err.current_context(),
            errors::ConnectorError::InvalidConnectorConfig {
                config: "metadata.store_name"
            }
        ));
    }

    #[test]
    fn metadata_without_store_name_returns_invalid_connector_config() {
        // Metadata present but missing the required `store_name` key must
        // also be rejected with the same InvalidConnectorConfig error.
        let meta = common_utils::pii::SecretSerdeValue::new(serde_json::json!({
            "unrelated": "value"
        }));
        let err = PaydunyaConnectorMetadataObject::try_from(Some(&meta))
            .expect_err("metadata without store_name must error");
        assert!(matches!(
            err.current_context(),
            errors::ConnectorError::InvalidConnectorConfig {
                config: "metadata.store_name"
            }
        ));
    }

    #[test]
    fn preprocessing_response_deserializes_success_envelope() {
        let body = r#"{
            "response_code": "00",
            "response_text": "Invoice Created",
            "description": "Test invoice",
            "token": "test_jkEdPY8SuG"
        }"#;

        let response: PaydunyaPaymentsPreProcessingResponse = serde_json::from_str(body).unwrap();
        assert_eq!(response.response_code, "00");
        assert_eq!(response.token.as_deref(), Some("test_jkEdPY8SuG"));
    }

    #[test]
    fn preprocessing_response_deserializes_failure_envelope_without_token() {
        // Paydunya returns HTTP 200 even on failure, with only
        // `response_code`/`response_text` and no `description`/`token`.
        let body = r#"{
            "response_code": "1001",
            "response_text": "Token invalide"
        }"#;

        let response: PaydunyaPaymentsPreProcessingResponse = serde_json::from_str(body).unwrap();
        assert_eq!(response.response_code, "1001");
        assert_eq!(response.token, None);
        assert_eq!(response.description, None);
    }

    // ---------------------------------------------------------------
    // PaydunyaPaymentsResponse JSON deserialization
    // ---------------------------------------------------------------

    #[test]
    fn softpay_response_message_is_optional() {
        // `message` may be absent; parsing must still succeed.
        let body = r#"{"success": true}"#;
        let parsed: PaydunyaPaymentsResponse = serde_json::from_str(body).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.message, None);
    }
}
