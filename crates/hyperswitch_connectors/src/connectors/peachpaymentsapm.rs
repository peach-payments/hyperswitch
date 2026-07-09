pub mod transformers;

use std::sync::LazyLock;

use common_enums::{enums, CaptureMethod};
use common_utils::{
    errors::{CryptoError, CustomResult},
    ext_traits::{ByteSliceExt, BytesExt},
    id_type,
    request::{Method, Request, RequestBuilder, RequestContent},
    types::{AmountConvertor, StringMajorUnit, StringMajorUnitForConnector},
};
use error_stack::ResultExt;
use hyperswitch_domain_models::{
    router_data::{AccessToken, ConnectorAuthType, ErrorResponse, RouterData},
    router_flow_types::{
        access_token_auth::AccessTokenAuth,
        payments::{Authorize, Capture, PSync, PaymentMethodToken, Session, SetupMandate, Void},
        refunds::{Execute, RSync},
    },
    router_request_types::{
        AccessTokenRequestData, PaymentMethodTokenizationData, PaymentsAuthorizeData,
        PaymentsCancelData, PaymentsCaptureData, PaymentsSessionData, PaymentsSyncData,
        RefundsData, SetupMandateRequestData,
    },
    router_response_types::{
        ConnectorInfo, PaymentMethodDetails, PaymentsResponseData, RefundsResponseData,
        SupportedPaymentMethods, SupportedPaymentMethodsExt,
    },
    types::{
        PaymentsAuthorizeRouterData, PaymentsCancelRouterData, PaymentsCaptureRouterData,
        PaymentsSyncRouterData, RefundSyncRouterData, RefundsRouterData,
    },
};
use hyperswitch_interfaces::{
    api::{
        self, ConnectorCommon, ConnectorCommonExt, ConnectorIntegration, ConnectorSpecifications,
        ConnectorValidation,
    },
    configs::Connectors,
    consts::{NO_ERROR_CODE, NO_ERROR_MESSAGE},
    errors,
    events::connector_api_logs::ConnectorEvent,
    types::{self, Response},
    webhooks,
};
use hyperswitch_masking::{PeekInterface, Secret};
use ring::aead::{self, UnboundKey};
use transformers as peachpaymentsapm;

use crate::{
    constants::headers,
    types::ResponseRouterData,
    utils::{self, RefundsRequestData},
};

const APM_WEBHOOK_IV_HEADER: &str = "X-Initialization-Vector";
const APM_WEBHOOK_AUTH_TAG_HEADER: &str = "X-Authentication-Tag";

#[derive(Clone)]
pub struct Peachpaymentsapm {
    amount_converter: &'static (dyn AmountConvertor<Output = StringMajorUnit> + Sync),
}

impl Peachpaymentsapm {
    pub fn new() -> &'static Self {
        &Self {
            amount_converter: &StringMajorUnitForConnector,
        }
    }

    /// Builds the Payments API status url; the Payments API only accepts
    /// authentication as query parameters on GET requests
    fn build_apm_status_url(
        &self,
        connectors: &Connectors,
        transaction_id: &str,
        auth_type: &ConnectorAuthType,
        connector_meta_data: &Option<common_utils::pii::SecretSerdeValue>,
    ) -> CustomResult<String, errors::ConnectorError> {
        let authentication = peachpaymentsapm::ApmAuthentication::try_from_connector_data(
            auth_type,
            connector_meta_data,
        )?;
        let mut url = url::Url::parse(&format!(
            "{}/payments/{}",
            self.base_url(connectors),
            transaction_id
        ))
        .change_context(errors::ConnectorError::RequestEncodingFailed)?;
        url.query_pairs_mut()
            .append_pair("authentication.entityId", authentication.entity_id.peek())
            .append_pair("authentication.userId", authentication.user_id.peek())
            .append_pair("authentication.password", authentication.password.peek());
        Ok(url.to_string())
    }
}

impl api::Payment for Peachpaymentsapm {}
impl api::PaymentSession for Peachpaymentsapm {}
impl api::ConnectorAccessToken for Peachpaymentsapm {}
impl api::MandateSetup for Peachpaymentsapm {}
impl api::PaymentAuthorize for Peachpaymentsapm {}
impl api::PaymentSync for Peachpaymentsapm {}
impl api::PaymentCapture for Peachpaymentsapm {}
impl api::PaymentVoid for Peachpaymentsapm {}
impl api::Refund for Peachpaymentsapm {}
impl api::RefundExecute for Peachpaymentsapm {}
impl api::RefundSync for Peachpaymentsapm {}
impl api::PaymentToken for Peachpaymentsapm {}

impl ConnectorIntegration<PaymentMethodToken, PaymentMethodTokenizationData, PaymentsResponseData>
    for Peachpaymentsapm
{
    // Not Implemented (R)
}

impl<Flow, Request, Response> ConnectorCommonExt<Flow, Request, Response> for Peachpaymentsapm
where
    Self: ConnectorIntegration<Flow, Request, Response>,
{
    fn build_headers(
        &self,
        _req: &RouterData<Flow, Request, Response>,
        _connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        // The Payments API authenticates in the request body/query, so only the
        // content-type header is required here
        Ok(vec![(
            headers::CONTENT_TYPE.to_string(),
            self.get_content_type().to_string().into(),
        )])
    }
}

impl ConnectorCommon for Peachpaymentsapm {
    fn id(&self) -> &'static str {
        "peachpaymentsapm"
    }

    fn get_currency_unit(&self) -> api::CurrencyUnit {
        // The Payments API accepts amounts as decimal strings in major units
        api::CurrencyUnit::Base
    }

    fn common_get_content_type(&self) -> &'static str {
        "application/json"
    }

    fn base_url<'a>(&self, connectors: &'a Connectors) -> &'a str {
        connectors.peachpaymentsapm.base_url.as_ref()
    }

    fn get_auth_header(
        &self,
        _auth_type: &ConnectorAuthType,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        // The Payments API authenticates via the `authentication` object in the
        // request body (or query parameters on GET), not via headers
        Ok(vec![])
    }

    fn build_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        // The Payments API rate-limits status queries (2 per minute per transaction);
        // treat 429 as still pending so the attempt is not failed prematurely
        if res.status_code == 429 {
            return Ok(ErrorResponse {
                status_code: res.status_code,
                code: "RATE_LIMITED".to_string(),
                message: "Too many requests, the payment is still being processed".to_string(),
                reason: Some(
                    "Rate limited by Peach Payments, status will be updated via webhook or the next sync"
                        .to_string(),
                ),
                attempt_status: Some(enums::AttemptStatus::Pending),
                connector_transaction_id: None,
                connector_response_reference_id: None,
                network_advice_code: None,
                network_decline_code: None,
                network_error_message: None,
                connector_metadata: None,
            });
        }

        // Payments API (APM) error shape
        if let Ok(response) = res
            .response
            .parse_struct::<peachpaymentsapm::PeachpaymentsApmErrorResponse>(
                "PeachpaymentsApmErrorResponse",
            )
        {
            event_builder.map(|i| i.set_response_body(&response));
            router_env::logger::info!(connector_response=?response);

            return Ok(ErrorResponse {
                status_code: res.status_code,
                code: response.result.code.clone(),
                message: response
                    .result
                    .description
                    .clone()
                    .unwrap_or(NO_ERROR_MESSAGE.to_string()),
                reason: peachpaymentsapm::build_apm_error_reason(&response.result),
                attempt_status: Some(peachpaymentsapm::map_apm_result_code_to_attempt_status(
                    &response.result.code,
                )),
                connector_transaction_id: response.id.clone(),
                connector_response_reference_id: response.merchant_transaction_id.clone(),
                network_advice_code: None,
                network_decline_code: None,
                network_error_message: None,
                connector_metadata: None,
            });
        }

        // Fallback for unrecognised bodies; never infer a status from raw text
        let raw_body = String::from_utf8_lossy(&res.response).to_string();
        router_env::logger::info!(connector_response=?raw_body);
        Ok(ErrorResponse {
            status_code: res.status_code,
            code: NO_ERROR_CODE.to_string(),
            message: NO_ERROR_MESSAGE.to_string(),
            reason: Some(raw_body),
            attempt_status: None,
            connector_transaction_id: None,
            connector_response_reference_id: None,
            network_advice_code: None,
            network_decline_code: None,
            network_error_message: None,
            connector_metadata: None,
        })
    }
}

impl ConnectorValidation for Peachpaymentsapm {
    fn validate_psync_reference_id(
        &self,
        _data: &PaymentsSyncData,
        _is_three_ds: bool,
        _status: enums::AttemptStatus,
        _connector_meta_data: Option<common_utils::pii::SecretSerdeValue>,
    ) -> CustomResult<(), errors::ConnectorError> {
        Ok(())
    }
}

impl ConnectorIntegration<Session, PaymentsSessionData, PaymentsResponseData> for Peachpaymentsapm {
    //TODO: implement sessions flow
}

impl ConnectorIntegration<AccessTokenAuth, AccessTokenRequestData, AccessToken>
    for Peachpaymentsapm
{
}

impl ConnectorIntegration<SetupMandate, SetupMandateRequestData, PaymentsResponseData>
    for Peachpaymentsapm
{
    fn build_request(
        &self,
        _req: &RouterData<SetupMandate, SetupMandateRequestData, PaymentsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Err(errors::ConnectorError::NotImplemented(
            "Setup Mandate flow for Peachpaymentsapm".to_string(),
        )
        .into())
    }
}

impl ConnectorIntegration<Authorize, PaymentsAuthorizeData, PaymentsResponseData>
    for Peachpaymentsapm
{
    fn get_headers(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        match req.request.capture_method.unwrap_or_default() {
            CaptureMethod::Automatic => Ok(format!("{}/payments", self.base_url(connectors))),
            CaptureMethod::Manual
            | CaptureMethod::ManualMultiple
            | CaptureMethod::Scheduled
            | CaptureMethod::SequentialAutomatic => {
                Err(errors::ConnectorError::CaptureMethodNotSupported.into())
            }
        }
    }

    fn get_request_body(
        &self,
        req: &PaymentsAuthorizeRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let amount = utils::convert_amount(
            self.amount_converter,
            req.request.minor_amount,
            req.request.currency,
        )?;

        let connector_router_data =
            peachpaymentsapm::PeachpaymentsApmRouterData::from((amount, req));
        let connector_req =
            peachpaymentsapm::PeachpaymentsApmPaymentsRequest::try_from(&connector_router_data)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&types::PaymentsAuthorizeType::get_url(
                    self, req, connectors,
                )?)
                .attach_default_headers()
                .headers(types::PaymentsAuthorizeType::get_headers(
                    self, req, connectors,
                )?)
                .set_body(types::PaymentsAuthorizeType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsAuthorizeRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsAuthorizeRouterData, errors::ConnectorError> {
        let response: peachpaymentsapm::PeachpaymentsApmPaymentsResponse = res
            .response
            .parse_struct("PeachpaymentsApmPaymentsAuthorizeResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }

    fn get_5xx_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<PSync, PaymentsSyncData, PaymentsResponseData> for Peachpaymentsapm {
    fn get_headers(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        let transaction_id = req
            .request
            .connector_transaction_id
            .get_connector_transaction_id()
            .change_context(errors::ConnectorError::MissingConnectorTransactionID)?;
        self.build_apm_status_url(
            connectors,
            &transaction_id,
            &req.connector_auth_type,
            &req.connector_meta_data,
        )
    }

    fn build_request(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Get)
                .url(&types::PaymentsSyncType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(types::PaymentsSyncType::get_headers(self, req, connectors)?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsSyncRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsSyncRouterData, errors::ConnectorError> {
        let response: peachpaymentsapm::PeachpaymentsApmPaymentsResponse = res
            .response
            .parse_struct("PeachpaymentsApmPaymentsSyncResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }

    fn get_5xx_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<Capture, PaymentsCaptureData, PaymentsResponseData> for Peachpaymentsapm {
    fn get_headers(
        &self,
        req: &PaymentsCaptureRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn build_request(
        &self,
        _req: &PaymentsCaptureRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        // The Payments API auto-captures APM payments; separate capture is not supported
        Err(errors::ConnectorError::FlowNotSupported {
            flow: "Capture".to_string(),
            connector: "Peachpaymentsapm".to_string(),
        }
        .into())
    }
}

impl ConnectorIntegration<Void, PaymentsCancelData, PaymentsResponseData> for Peachpaymentsapm {
    fn get_headers(
        &self,
        req: &PaymentsCancelRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn build_request(
        &self,
        _req: &PaymentsCancelRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        // The Payments API does not support voiding APM payments
        Err(errors::ConnectorError::FlowNotSupported {
            flow: "Void".to_string(),
            connector: "Peachpaymentsapm".to_string(),
        }
        .into())
    }
}

impl ConnectorIntegration<Execute, RefundsData, RefundsResponseData> for Peachpaymentsapm {
    fn get_headers(
        &self,
        req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!(
            "{}/payments/{}",
            self.base_url(connectors),
            req.request.connector_transaction_id
        ))
    }

    fn get_request_body(
        &self,
        req: &RefundsRouterData<Execute>,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let amount = utils::convert_amount(
            self.amount_converter,
            req.request.minor_refund_amount,
            req.request.currency,
        )?;
        let connector_router_data =
            peachpaymentsapm::PeachpaymentsApmRouterData::from((amount, req));
        let connector_req =
            peachpaymentsapm::PeachpaymentsApmRefundRequest::try_from(&connector_router_data)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        let request = RequestBuilder::new()
            .method(Method::Post)
            .url(&types::RefundExecuteType::get_url(self, req, connectors)?)
            .attach_default_headers()
            .headers(types::RefundExecuteType::get_headers(
                self, req, connectors,
            )?)
            .set_body(types::RefundExecuteType::get_request_body(
                self, req, connectors,
            )?)
            .build();
        Ok(Some(request))
    }

    fn handle_response(
        &self,
        data: &RefundsRouterData<Execute>,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RefundsRouterData<Execute>, errors::ConnectorError> {
        let response: peachpaymentsapm::PeachpaymentsApmPaymentsResponse = res
            .response
            .parse_struct("PeachpaymentsApmRefundResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }

    fn get_5xx_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<RSync, RefundsData, RefundsResponseData> for Peachpaymentsapm {
    fn get_headers(
        &self,
        req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_url(
        &self,
        req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        let connector_refund_id = req.request.get_connector_refund_id()?;
        self.build_apm_status_url(
            connectors,
            &connector_refund_id,
            &req.connector_auth_type,
            &req.connector_meta_data,
        )
    }

    fn build_request(
        &self,
        req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Get)
                .url(&types::RefundSyncType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(types::RefundSyncType::get_headers(self, req, connectors)?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &RefundSyncRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RefundSyncRouterData, errors::ConnectorError> {
        let response: peachpaymentsapm::PeachpaymentsApmPaymentsResponse = res
            .response
            .parse_struct("PeachpaymentsApmRsyncResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }

    fn get_5xx_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

/// Decrypts an AES-GCM encrypted Payments API webhook where the IV, auth
/// tag, and ciphertext are provided separately as hex strings (key from the
/// dashboard, IV and auth tag from the webhook request headers).
///
/// The Payments API documents AES-128-GCM (16 byte key); the cipher is picked
/// from the configured key length so a 32 byte key also keeps working.
fn decrypt_apm_webhook_payload(
    hex_key: &str,
    hex_iv: &str,
    hex_auth_tag: &str,
    hex_encrypted_body: &str,
) -> CustomResult<Vec<u8>, CryptoError> {
    let key_bytes = hex::decode(hex_key)
        .change_context(CryptoError::DecodingFailed)
        .attach_printable("Failed to decode hex key")?;
    let iv_bytes = hex::decode(hex_iv)
        .change_context(CryptoError::DecodingFailed)
        .attach_printable("Failed to decode hex IV")?;
    let auth_tag_bytes = hex::decode(hex_auth_tag)
        .change_context(CryptoError::DecodingFailed)
        .attach_printable("Failed to decode hex auth tag")?;
    let encrypted_body_bytes = hex::decode(hex_encrypted_body)
        .change_context(CryptoError::DecodingFailed)
        .attach_printable("Failed to decode hex encrypted body")?;
    let algorithm = match key_bytes.len() {
        16 => &aead::AES_128_GCM,
        32 => &aead::AES_256_GCM,
        _ => {
            return Err(CryptoError::InvalidKeyLength)
                .attach_printable("Key must be 16 bytes (AES-128-GCM) or 32 bytes (AES-256-GCM)");
        }
    };
    if iv_bytes.len() != aead::NONCE_LEN {
        return Err(CryptoError::InvalidIvLength)
            .attach_printable(format!("IV must be {} bytes for AES-GCM", aead::NONCE_LEN));
    }
    if auth_tag_bytes.len() != 16 {
        return Err(CryptoError::InvalidTagLength)
            .attach_printable("Auth tag must be 16 bytes for AES-GCM");
    }

    let unbound_key = UnboundKey::new(algorithm, &key_bytes)
        .change_context(CryptoError::DecodingFailed)
        .attach_printable("Failed to create unbound key")?;

    let less_safe_key = aead::LessSafeKey::new(unbound_key);

    let nonce_arr: [u8; aead::NONCE_LEN] = iv_bytes
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::InvalidIvLength)?;
    let nonce = aead::Nonce::assume_unique_for_key(nonce_arr);

    let mut ciphertext_and_tag = encrypted_body_bytes;
    ciphertext_and_tag.extend_from_slice(&auth_tag_bytes);

    less_safe_key
        .open_in_place(nonce, aead::Aad::empty(), &mut ciphertext_and_tag)
        .change_context(CryptoError::DecodingFailed)
        .attach_printable("Failed to decrypt webhook payload")?;

    let original_ciphertext_len = ciphertext_and_tag.len() - auth_tag_bytes.len();
    ciphertext_and_tag.truncate(original_ciphertext_len);

    Ok(ciphertext_and_tag)
}

impl Peachpaymentsapm {
    fn decrypt_apm_webhook(
        &self,
        request: &webhooks::IncomingWebhookRequestDetails<'_>,
        connector_webhook_secrets: &api_models::webhooks::ConnectorWebhookSecrets,
    ) -> CustomResult<Vec<u8>, errors::ConnectorError> {
        let key_hex = String::from_utf8(connector_webhook_secrets.secret.to_vec())
            .map_err(|_| errors::ConnectorError::WebhookVerificationSecretInvalid)
            .attach_printable("Peachpaymentsapm webhook secret is not a valid UTF-8 string")?;

        let iv_hex = request
            .headers
            .get(APM_WEBHOOK_IV_HEADER)
            .ok_or(errors::ConnectorError::WebhookBodyDecodingFailed)
            .attach_printable("Missing X-Initialization-Vector header")?
            .to_str()
            .map_err(|_| errors::ConnectorError::WebhookBodyDecodingFailed)
            .attach_printable("Invalid X-Initialization-Vector header value")?;

        let auth_tag_hex = request
            .headers
            .get(APM_WEBHOOK_AUTH_TAG_HEADER)
            .ok_or(errors::ConnectorError::WebhookBodyDecodingFailed)
            .attach_printable("Missing X-Authentication-Tag header")?
            .to_str()
            .map_err(|_| errors::ConnectorError::WebhookBodyDecodingFailed)
            .attach_printable("Invalid X-Authentication-Tag header value")?;

        let body = String::from_utf8(request.body.to_vec())
            .map_err(|_| errors::ConnectorError::WebhookBodyDecodingFailed)
            .attach_printable("Failed to read encrypted webhook body as UTF-8")?;
        // The encrypted payload is either raw hex or wrapped as {"encryptedBody": "<hex>"}
        let encrypted_body_hex =
            serde_json::from_str::<peachpaymentsapm::ApmEncryptedWebhookBody>(&body)
                .map(|wrapped_body| wrapped_body.encrypted_body)
                .unwrap_or_else(|_| body.trim().to_string());

        decrypt_apm_webhook_payload(&key_hex, iv_hex, auth_tag_hex, &encrypted_body_hex)
            .change_context(errors::ConnectorError::WebhookBodyDecodingFailed)
            .attach_printable("Failed to decrypt Peachpaymentsapm APM webhook payload")
    }
}

#[async_trait::async_trait]
impl webhooks::IncomingWebhook for Peachpaymentsapm {
    async fn decode_webhook_body(
        &self,
        request: &webhooks::IncomingWebhookRequestDetails<'_>,
        merchant_id: &id_type::MerchantId,
        connector_webhook_details: Option<common_utils::pii::SecretSerdeValue>,
        connector_name: &str,
    ) -> CustomResult<Vec<u8>, errors::ConnectorError> {
        let connector_webhook_secrets = self
            .get_webhook_source_verification_merchant_secret(
                merchant_id,
                connector_name,
                connector_webhook_details,
            )
            .await
            .change_context(errors::ConnectorError::WebhookSourceVerificationFailed)?;
        self.decrypt_apm_webhook(request, &connector_webhook_secrets)
    }

    fn get_webhook_object_reference_id(
        &self,
        request: &webhooks::IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<api_models::webhooks::ObjectReferenceId, errors::ConnectorError> {
        let webhook_body: peachpaymentsapm::PeachpaymentsApmWebhook = request
            .body
            .parse_struct("PeachpaymentsApmWebhook")
            .change_context(errors::ConnectorError::WebhookBodyDecodingFailed)?;

        if webhook_body.payment_type == Some(peachpaymentsapm::PeachApmPaymentType::RF) {
            Ok(api_models::webhooks::ObjectReferenceId::RefundId(
                api_models::webhooks::RefundIdType::ConnectorRefundId(webhook_body.id),
            ))
        } else {
            Ok(api_models::webhooks::ObjectReferenceId::PaymentId(
                api_models::payments::PaymentIdType::ConnectorTransactionId(webhook_body.id),
            ))
        }
    }

    fn get_webhook_event_type(
        &self,
        request: &webhooks::IncomingWebhookRequestDetails<'_>,
        _context: Option<&webhooks::WebhookContext>,
    ) -> CustomResult<api_models::webhooks::IncomingWebhookEvent, errors::ConnectorError> {
        let webhook_body: peachpaymentsapm::PeachpaymentsApmWebhook = request
            .body
            .parse_struct("PeachpaymentsApmWebhook")
            .change_context(errors::ConnectorError::WebhookBodyDecodingFailed)?;

        let is_refund =
            webhook_body.payment_type == Some(peachpaymentsapm::PeachApmPaymentType::RF);
        let code = webhook_body.result.code.as_str();

        if code.starts_with("000.000.") || code.starts_with("000.100.1") {
            if is_refund {
                Ok(api_models::webhooks::IncomingWebhookEvent::RefundSuccess)
            } else {
                Ok(api_models::webhooks::IncomingWebhookEvent::PaymentIntentSuccess)
            }
        } else if code.starts_with("000.200") {
            if is_refund {
                Ok(api_models::webhooks::IncomingWebhookEvent::EventNotSupported)
            } else {
                Ok(api_models::webhooks::IncomingWebhookEvent::PaymentIntentProcessing)
            }
        } else if code.starts_with("100.")
            || code.starts_with("200.")
            || code.starts_with("800.")
            || code.starts_with("900.")
        {
            if is_refund {
                Ok(api_models::webhooks::IncomingWebhookEvent::RefundFailure)
            } else {
                Ok(api_models::webhooks::IncomingWebhookEvent::PaymentIntentFailure)
            }
        } else {
            Ok(api_models::webhooks::IncomingWebhookEvent::EventNotSupported)
        }
    }

    fn get_webhook_resource_object(
        &self,
        request: &webhooks::IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<Box<dyn hyperswitch_masking::ErasedMaskSerialize>, errors::ConnectorError>
    {
        let webhook_body: peachpaymentsapm::PeachpaymentsApmWebhook = request
            .body
            .parse_struct("PeachpaymentsApmWebhook")
            .change_context(errors::ConnectorError::WebhookBodyDecodingFailed)?;

        Ok(Box::new(webhook_body))
    }

    async fn verify_webhook_source(
        &self,
        request: &webhooks::IncomingWebhookRequestDetails<'_>,
        merchant_id: &id_type::MerchantId,
        connector_webhook_details: Option<common_utils::pii::SecretSerdeValue>,
        _connector_account_details: common_utils::crypto::Encryptable<Secret<serde_json::Value>>,
        connector_name: &str,
    ) -> CustomResult<bool, errors::ConnectorError> {
        // A successful AES-GCM decryption authenticates the webhook source,
        // since the tag verification fails for any other key
        let connector_webhook_secrets = self
            .get_webhook_source_verification_merchant_secret(
                merchant_id,
                connector_name,
                connector_webhook_details,
            )
            .await
            .change_context(errors::ConnectorError::WebhookSourceVerificationFailed)?;
        if self
            .decrypt_apm_webhook(request, &connector_webhook_secrets)
            .is_ok()
        {
            return Ok(true);
        }
        // The body has usually already been decrypted by decode_webhook_body
        // by the time source verification runs; reaching this point with a
        // valid plaintext payload means the AES-GCM decryption succeeded
        Ok(request
            .body
            .parse_struct::<peachpaymentsapm::PeachpaymentsApmWebhook>("PeachpaymentsApmWebhook")
            .is_ok())
    }
}

static PEACHPAYMENTSAPM_SUPPORTED_PAYMENT_METHODS: LazyLock<SupportedPaymentMethods> =
    LazyLock::new(|| {
        let mut peachpayments_supported_payment_methods = SupportedPaymentMethods::new();

        // APMs via the Payments API support automatic capture only
        let apm_details = PaymentMethodDetails {
            mandates: enums::FeatureStatus::NotSupported,
            refunds: enums::FeatureStatus::Supported,
            supported_capture_methods: vec![CaptureMethod::Automatic],
            specific_features: None,
        };

        for payment_method_type in [
            enums::PaymentMethodType::CapitecPay,
            enums::PaymentMethodType::PayShap,
            enums::PaymentMethodType::NedbankDirectEft,
            enums::PaymentMethodType::PeachEft,
        ] {
            peachpayments_supported_payment_methods.add(
                enums::PaymentMethod::BankTransfer,
                payment_method_type,
                apm_details.clone(),
            );
        }

        for payment_method_type in [
            enums::PaymentMethodType::Payflex,
            enums::PaymentMethodType::ZeroPay,
            enums::PaymentMethodType::Float,
            enums::PaymentMethodType::HappyPay,
            enums::PaymentMethodType::Mobicred,
            enums::PaymentMethodType::Rcs,
            enums::PaymentMethodType::APlus,
        ] {
            peachpayments_supported_payment_methods.add(
                enums::PaymentMethod::PayLater,
                payment_method_type,
                apm_details.clone(),
            );
        }

        for payment_method_type in [
            enums::PaymentMethodType::Mpesa,
            enums::PaymentMethodType::BlinkByEmtel,
            enums::PaymentMethodType::McbJuice,
            enums::PaymentMethodType::ScanToPay,
            enums::PaymentMethodType::Maucas,
        ] {
            peachpayments_supported_payment_methods.add(
                enums::PaymentMethod::Wallet,
                payment_method_type,
                apm_details.clone(),
            );
        }

        peachpayments_supported_payment_methods.add(
            enums::PaymentMethod::Voucher,
            enums::PaymentMethodType::OneForYou,
            apm_details.clone(),
        );

        peachpayments_supported_payment_methods.add(
            enums::PaymentMethod::Crypto,
            enums::PaymentMethodType::MoneyBadger,
            apm_details,
        );

        peachpayments_supported_payment_methods
    });

static PEACHPAYMENTS_CONNECTOR_INFO: ConnectorInfo = ConnectorInfo {
    display_name: "Peach Payments APM",
    description: "Alternative payment methods for Peach Payments via the Payments API, including PayShap, Capitec Pay, EFT, buy-now-pay-later, wallets and vouchers.",
    connector_type: enums::HyperswitchConnectorCategory::PaymentGateway,
    integration_status: enums::ConnectorIntegrationStatus::Live,
};

static PEACHPAYMENTS_SUPPORTED_WEBHOOK_FLOWS: [enums::EventClass; 2] =
    [enums::EventClass::Payments, enums::EventClass::Refunds];

impl ConnectorSpecifications for Peachpaymentsapm {
    fn get_connector_about(&self) -> Option<&'static ConnectorInfo> {
        Some(&PEACHPAYMENTS_CONNECTOR_INFO)
    }

    fn get_supported_payment_methods(&self) -> Option<&'static SupportedPaymentMethods> {
        Some(&*PEACHPAYMENTSAPM_SUPPORTED_PAYMENT_METHODS)
    }

    fn get_supported_webhook_flows(&self) -> Option<&'static [enums::EventClass]> {
        Some(&PEACHPAYMENTS_SUPPORTED_WEBHOOK_FLOWS)
    }

    #[cfg(feature = "v1")]
    fn generate_connector_request_reference_id(
        &self,
        _payment_intent: &hyperswitch_domain_models::payments::PaymentIntent,
        _payment_attempt: &hyperswitch_domain_models::payments::payment_attempt::PaymentAttempt,
        _is_config_enabled_to_send_payment_id_as_connector_request_id: bool,
    ) -> String {
        // The Payments API requires merchantTransactionId to be 8-16
        // alphanumeric characters; APM transactions correlate on the Peach
        // unique id, so a generated reference is sufficient
        utils::generate_alphanumeric_code(16, 16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encrypt_for_test(
        algorithm: &'static aead::Algorithm,
        key_bytes: &[u8],
        iv_bytes: [u8; aead::NONCE_LEN],
        plaintext: &[u8],
    ) -> (Vec<u8>, Vec<u8>) {
        let unbound_key = UnboundKey::new(algorithm, key_bytes).expect("failed to create key");
        let sealing_key = aead::LessSafeKey::new(unbound_key);
        let nonce = aead::Nonce::assume_unique_for_key(iv_bytes);
        let mut ciphertext = plaintext.to_vec();
        let tag = sealing_key
            .seal_in_place_separate_tag(nonce, aead::Aad::empty(), &mut ciphertext)
            .expect("failed to encrypt");
        (ciphertext, tag.as_ref().to_vec())
    }

    // The Payments API documents AES-128-GCM webhook encryption
    #[test]
    fn test_decrypt_apm_webhook_payload_aes_128_round_trip() {
        let key_bytes = [0x42_u8; 16];
        let iv_bytes = [0x24_u8; aead::NONCE_LEN];
        let plaintext = br#"{"id":"8ac7a4a09c8b9c8e019c8ba47c0000aa","paymentType":"DB","result":{"code":"000.000.000","description":"Transaction succeeded"}}"#;

        let (ciphertext, tag) =
            encrypt_for_test(&aead::AES_128_GCM, &key_bytes, iv_bytes, plaintext);

        let decrypted = decrypt_apm_webhook_payload(
            &hex::encode(key_bytes),
            &hex::encode(iv_bytes),
            &hex::encode(tag),
            &hex::encode(&ciphertext),
        )
        .expect("failed to decrypt");

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_apm_webhook_payload_aes_256_round_trip() {
        let key_bytes = [0x42_u8; 32];
        let iv_bytes = [0x24_u8; aead::NONCE_LEN];
        let plaintext = br#"{"id":"8ac7a4a09c8b9c8e019c8ba47c0000aa","paymentType":"DB","result":{"code":"000.000.000","description":"Transaction succeeded"}}"#;

        let (ciphertext, tag) =
            encrypt_for_test(&aead::AES_256_GCM, &key_bytes, iv_bytes, plaintext);

        let decrypted = decrypt_apm_webhook_payload(
            &hex::encode(key_bytes),
            &hex::encode(iv_bytes),
            &hex::encode(tag),
            &hex::encode(&ciphertext),
        )
        .expect("failed to decrypt");

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_apm_webhook_payload_rejects_wrong_key() {
        let key_bytes = [0x42_u8; 32];
        let wrong_key_bytes = [0x43_u8; 32];
        let iv_bytes = [0x24_u8; aead::NONCE_LEN];
        let plaintext = b"payload";

        let unbound_key =
            UnboundKey::new(&aead::AES_256_GCM, &key_bytes).expect("failed to create key");
        let sealing_key = aead::LessSafeKey::new(unbound_key);
        let nonce = aead::Nonce::assume_unique_for_key(iv_bytes);
        let mut ciphertext = plaintext.to_vec();
        let tag = sealing_key
            .seal_in_place_separate_tag(nonce, aead::Aad::empty(), &mut ciphertext)
            .expect("failed to encrypt");

        assert!(decrypt_apm_webhook_payload(
            &hex::encode(wrong_key_bytes),
            &hex::encode(iv_bytes),
            &hex::encode(tag.as_ref()),
            &hex::encode(&ciphertext),
        )
        .is_err());
    }
}
