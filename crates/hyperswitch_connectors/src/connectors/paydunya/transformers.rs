use common_enums::enums;
use common_utils::types::MinorUnit;
use hyperswitch_domain_models::{
    payment_method_data::PaymentMethodData,
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
    utils::PaymentsPreProcessingRequestData,
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
        Ok(Self {
            status,
            description: Some(item.response.description),
            response: Ok(PaymentsResponseData::PreProcessingResponse {
                pre_processing_id: PreprocessingResponseId::PreProcessingId(item.response.token),
                connector_metadata: None,
                session_token: None,
                connector_response_reference_id: None,
            }),
            ..item.data
        })
    }
}

//TODO: Fill the struct with respective fields
#[derive(Default, Debug, Serialize, PartialEq)]
pub struct PaydunyaPaymentsRequest {
    amount: MinorUnit,
    card: PaydunyaCard,
}

#[derive(Default, Debug, Serialize, Eq, PartialEq)]
pub struct PaydunyaCard {
    number: cards::CardNumber,
    expiry_month: Secret<String>,
    expiry_year: Secret<String>,
    cvc: Secret<String>,
    complete: bool,
}

impl TryFrom<&PaydunyaRouterData<&PaymentsAuthorizeRouterData>> for PaydunyaPaymentsRequest {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: &PaydunyaRouterData<&PaymentsAuthorizeRouterData>,
    ) -> Result<Self, Self::Error> {
        match item.router_data.request.payment_method_data.clone() {
            PaymentMethodData::Card(_) => Err(errors::ConnectorError::NotImplemented(
                "Card payment method not implemented".to_string(),
            )
            .into()),
            _ => Err(errors::ConnectorError::NotImplemented("Payment method".to_string()).into()),
        }
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
