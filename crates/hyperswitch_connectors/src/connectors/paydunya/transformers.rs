use common_enums::enums;
use common_utils::{pii::Email, types::MinorUnit};
use hyperswitch_domain_models::{
    router_data::{ConnectorAuthType, RouterData},
    router_flow_types::refunds::{Execute, RSync},
    router_request_types::ResponseId,
    router_response_types::{PaymentsResponseData, PreprocessingResponseId, RefundsResponseData},
    types::{PaymentsAuthorizeRouterData, PaymentsPreProcessingRouterData, RefundsRouterData},
};
use hyperswitch_interfaces::errors;
use hyperswitch_masking::Secret;
use serde::{Deserialize, Serialize};

use crate::{
    types::{
        PaymentsPreprocessingResponseRouterData, RefundsResponseRouterData, ResponseRouterData,
    },
    utils::{AddressDetailsData, PaymentsAuthorizeRequestData, PaymentsPreProcessingRequestData, RouterData as _},
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

impl TryFrom<&PaymentsPreProcessingRouterData> for PaydunyaPreprocessingRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(item: &PaymentsPreProcessingRouterData) -> Result<Self, Self::Error> {
        Ok(Self {
            invoice: Invoice {
                total_amount: item.request.get_minor_amount(),
            },
            store: Store {
                name: String::from("name"),
            },
            actions: Actions {
                callback_url: String::from("callback_url"),
                return_url: String::from("return_url"),
            },
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaydunyaPaymentsPreProcessingResponse {
    pub response_code: String,
    pub response_text: String,
    pub description: String,
    pub token: String,
}

impl TryFrom<PaymentsPreprocessingResponseRouterData<PaydunyaPaymentsPreProcessingResponse>>
    for PaymentsPreProcessingRouterData
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: PaymentsPreprocessingResponseRouterData<PaydunyaPaymentsPreProcessingResponse>,
    ) -> Result<Self, Self::Error> {
        let status = match item.response.response_code.as_str() {
            "00" => enums::AttemptStatus::AuthenticationSuccessful,
            _ => enums::AttemptStatus::AuthenticationFailed,
        };
        let token = item.response.token;
        Ok(Self {
            status,
            description: Some(item.response.description),
            // Persist the invoice token on the RouterData so that the subsequent
            // Authorize flow can read it via `router_data.preprocessing_id` and
            // pass it as `payment_token` to the SOFTPAY endpoint.
            preprocessing_id: Some(token.clone()),
            response: Ok(PaymentsResponseData::PreProcessingResponse {
                pre_processing_id: PreprocessingResponseId::PreProcessingId(token),
                connector_metadata: None,
                session_token: None,
                connector_response_reference_id: None,
            }),
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
    OrangeMoneyCi,
    OrangeMoneySenegal,
    WaveSenegal,
    WaveCi,
    FreeMoneySenegal,
    ExpressoSenegal,
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
            Self::OrangeMoneyCi => "softpay/orange-money-ci",
            Self::OrangeMoneySenegal => "softpay/new-orange-money-senegal",
            Self::WaveSenegal => "softpay/wave-senegal",
            Self::WaveCi => "softpay/wave-ci",
            Self::FreeMoneySenegal => "softpay/free-money-senegal",
            Self::ExpressoSenegal => "softpay/expresso-senegal",
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
}

impl TryFrom<&PaymentsAuthorizeRouterData> for PaydunyaOperator {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(item: &PaymentsAuthorizeRouterData) -> Result<Self, Self::Error> {
        // Hyperswitch does not (yet) carry first-class enum variants for Wave
        // or MTN Benin / MTN CI / etc., so we derive the Paydunya operator
        // from `(payment_method_type, billing.country)`. As new variants get
        // added to `PaymentMethodType`, prefer matching on those directly.
        let pm_type = item.request.payment_method_type;
        let country = item.get_optional_billing_country();

        match (pm_type, country) {
            // MTN family — typed as MoMo upstream
            (Some(enums::PaymentMethodType::Momo), Some(enums::CountryAlpha2::BJ)) => {
                Ok(Self::MtnBenin)
            }
            (Some(enums::PaymentMethodType::Momo), Some(enums::CountryAlpha2::CI)) => {
                Ok(Self::MtnCi)
            }
            (Some(enums::PaymentMethodType::Momo), Some(enums::CountryAlpha2::CM)) => {
                Ok(Self::MtnCameroun)
            }
            // Default MoMo to MTN Benin if the country isn't supplied — matches
            // the canonical SOFTPAY example used during integration.
            (Some(enums::PaymentMethodType::Momo), _) => Ok(Self::MtnBenin),

            // Moov family
            (Some(enums::PaymentMethodType::MobilePay), Some(enums::CountryAlpha2::BJ)) => {
                Ok(Self::MoovBenin)
            }
            (Some(enums::PaymentMethodType::MobilePay), Some(enums::CountryAlpha2::CI)) => {
                Ok(Self::MoovCi)
            }

            // Wave family — no dedicated PaymentMethodType yet, route by
            // country when the upstream type is MbWay (closest mobile-wallet
            // proxy in the current enum).
            (Some(enums::PaymentMethodType::MbWay), Some(enums::CountryAlpha2::SN)) => {
                Ok(Self::WaveSenegal)
            }
            (Some(enums::PaymentMethodType::MbWay), Some(enums::CountryAlpha2::CI)) => {
                Ok(Self::WaveCi)
            }

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
    OrangeMoneyCi(PaydunyaOrangeMoneyCiRequest),
    OrangeMoneySenegal(PaydunyaOrangeMoneySenegalRequest),
    WaveSenegal(PaydunyaWaveSenegalRequest),
    WaveCi(PaydunyaWaveCiRequest),
    FreeMoneySenegal(PaydunyaFreeMoneySenegalRequest),
    ExpressoSenegal(PaydunyaExpressoSenegalRequest),
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

/// Fields common to every SOFTPAY operator: payer identity, contact info and
/// the invoice token returned by the preprocessing flow.
struct CommonAuthorizeFields {
    full_name: Secret<String>,
    email: Email,
    phone_number: Secret<String>,
    payment_token: String,
}

impl CommonAuthorizeFields {
    fn try_from_router_data(
        item: &PaydunyaRouterData<&PaymentsAuthorizeRouterData>,
    ) -> Result<Self, error_stack::Report<errors::ConnectorError>> {
        let router_data = item.router_data;

        // The SOFTPAY payment_token must be the invoice token returned by the
        // checkout-invoice/create preprocessing call, which we stash on
        // `RouterData.preprocessing_id`.
        let payment_token = router_data.preprocessing_id.clone().ok_or(
            errors::ConnectorError::MissingConnectorRelatedTransactionID {
                id: "payment_token (paydunya invoice token from preprocessing)".to_string(),
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

        let phone_number = router_data.get_billing_phone_number()?;
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
                mtn_benin_wallet_provider: operator
                    .wallet_provider()
                    .unwrap_or("MTNBENIN"),
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
                mtn_cameroun_wallet_provider: operator
                    .wallet_provider()
                    .unwrap_or("MTNCAMEROUN"),
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
            PaydunyaOperator::OrangeMoneyCi => {
                // Orange Money CI is the only SOFTPAY flow that needs an OTP
                // generated upfront via USSD; we don't have a transport for
                // that in the current authorize request shape.
                return Err(errors::ConnectorError::NotImplemented(
                    "Paydunya Orange Money CI authorize (OTP collection not wired)".to_string(),
                )
                .into());
            }
            PaydunyaOperator::OrangeMoneySenegal => {
                Self::OrangeMoneySenegal(PaydunyaOrangeMoneySenegalRequest {
                    customer_name: common.full_name,
                    customer_email: common.email,
                    phone_number: common.phone_number,
                    invoice_token: common.payment_token,
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
        };

        Ok(request)
    }
}

//TODO: Fill the struct with respective fields
// Auth Struct
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
                key1,
                api_secret,
            } => Ok(Self {
                master_key: api_key.to_owned(),
                private_key: key1.to_owned(),
                token: api_secret.to_owned(),
            }),
            _ => Err(errors::ConnectorError::FailedToObtainAuthType.into()),
        }
    }
}
// PaymentsResponse
//TODO: Append the remaining status flags
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PaydunyaPaymentStatus {
    Succeeded,
    Failed,
    #[default]
    Processing,
}

impl From<PaydunyaPaymentStatus> for common_enums::AttemptStatus {
    fn from(item: PaydunyaPaymentStatus) -> Self {
        match item {
            PaydunyaPaymentStatus::Succeeded => Self::Charged,
            PaydunyaPaymentStatus::Failed => Self::Failure,
            PaydunyaPaymentStatus::Processing => Self::Authorizing,
        }
    }
}

//TODO: Fill the struct with respective fields
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaydunyaPaymentsResponse {
    status: PaydunyaPaymentStatus,
    id: String,
}

impl<F, T> TryFrom<ResponseRouterData<F, PaydunyaPaymentsResponse, T, PaymentsResponseData>>
    for RouterData<F, T, PaymentsResponseData>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: ResponseRouterData<F, PaydunyaPaymentsResponse, T, PaymentsResponseData>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            status: common_enums::AttemptStatus::from(item.response.status),
            response: Ok(PaymentsResponseData::TransactionResponse {
                resource_id: ResponseId::ConnectorTransactionId(item.response.id),
                redirection_data: Box::new(None),
                mandate_reference: Box::new(None),
                connector_metadata: None,
                network_txn_id: None,
                connector_response_reference_id: None,
                incremental_authorization_allowed: None,
                authentication_data: None,
                charges: None,
            }),
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

//TODO: Fill the struct with respective fields
#[derive(Default, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaydunyaErrorResponse {
    pub status_code: u16,
    pub code: String,
    pub message: String,
    pub reason: Option<String>,
    pub network_advice_code: Option<String>,
    pub network_decline_code: Option<String>,
    pub network_error_message: Option<String>,
}
