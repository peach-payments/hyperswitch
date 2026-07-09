use std::collections::HashMap;

use common_utils::{
    pii,
    request::Method,
    types::{StringMajorUnit, StringMajorUnitForConnector},
};
use error_stack::ResultExt;
use hyperswitch_domain_models::{
    payment_method_data::{
        BankTransferData, PayLaterData, PaymentMethodData, VoucherData, WalletData,
    },
    router_data::{ConnectorAuthType, ErrorResponse, RouterData},
    router_flow_types::refunds::{Execute, RSync},
    router_request_types::{RefundsData, ResponseId},
    router_response_types::{PaymentsResponseData, RedirectForm, RefundsResponseData},
    types::{PaymentsAuthorizeRouterData, RefundsRouterData},
};
use hyperswitch_interfaces::{consts::NO_ERROR_MESSAGE, errors};
use hyperswitch_masking::{ExposeInterface, PeekInterface, Secret};
use serde::{Deserialize, Serialize};

use crate::{
    types::ResponseRouterData,
    utils::{self, PhoneDetailsData, RouterData as OtherRouterData},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct PeachpaymentsapmConnectorMetadataObject {
    /// The Payments API entity id, required for APM payments
    pub entity_id: Secret<String>,
}

impl TryFrom<&Option<pii::SecretSerdeValue>> for PeachpaymentsapmConnectorMetadataObject {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(meta_data: &Option<pii::SecretSerdeValue>) -> Result<Self, Self::Error> {
        let metadata = utils::to_connector_meta_from_secret::<Self>(meta_data.clone())
            .change_context(errors::ConnectorError::InvalidConnectorConfig {
                config: "metadata",
            })?;
        Ok(metadata)
    }
}

// Auth Struct for the Payments API (APMs)
pub struct PeachpaymentsapmAuthType {
    pub(crate) user_id: Secret<String>,
    pub(crate) password: Secret<String>,
}

impl TryFrom<&ConnectorAuthType> for PeachpaymentsapmAuthType {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(auth_type: &ConnectorAuthType) -> Result<Self, Self::Error> {
        if let ConnectorAuthType::BodyKey { api_key, key1 } = auth_type {
            Ok(Self {
                user_id: api_key.clone(),
                password: key1.clone(),
            })
        } else {
            Err(errors::ConnectorError::FailedToObtainAuthType)?
        }
    }
}

// ===========================================================================
// Payments API (APM) types
//
// Alternative payment methods are processed by the Peach Payments "Payments
// API". Amounts are decimal strings in major units and requests are
// authenticated with an `authentication` object in the body.
// ===========================================================================

pub struct PeachpaymentsApmRouterData<T> {
    pub amount: StringMajorUnit,
    pub router_data: T,
}

impl<T> From<(StringMajorUnit, T)> for PeachpaymentsApmRouterData<T> {
    fn from((amount, item): (StringMajorUnit, T)) -> Self {
        Self {
            amount,
            router_data: item,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmAuthentication {
    pub entity_id: Secret<String>,
    pub user_id: Secret<String>,
    pub password: Secret<String>,
}

impl ApmAuthentication {
    pub fn try_from_connector_data(
        auth_type: &ConnectorAuthType,
        connector_meta_data: &Option<pii::SecretSerdeValue>,
    ) -> Result<Self, error_stack::Report<errors::ConnectorError>> {
        let auth = PeachpaymentsapmAuthType::try_from(auth_type)?;
        let metadata = PeachpaymentsapmConnectorMetadataObject::try_from(connector_meta_data)?;
        Ok(Self {
            entity_id: metadata.entity_id,
            user_id: auth.user_id,
            password: auth.password,
        })
    }
}

/// The `paymentBrand` identifiers accepted by the Payments API
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PeachPaymentBrand {
    #[serde(rename = "CAPITECPAY")]
    CapitecPay,
    #[serde(rename = "PAYSHAP")]
    PayShap,
    #[serde(rename = "NEDBANKDIRECTEFT")]
    NedbankDirectEft,
    #[serde(rename = "PEACHEFT")]
    PeachEft,
    #[serde(rename = "PAYFLEX")]
    Payflex,
    #[serde(rename = "ZEROPAY")]
    ZeroPay,
    #[serde(rename = "FLOAT")]
    Float,
    #[serde(rename = "HAPPYPAY")]
    HappyPay,
    #[serde(rename = "MOBICRED")]
    Mobicred,
    #[serde(rename = "RCS")]
    Rcs,
    /// A+ Store Cards
    #[serde(rename = "APLUS")]
    APlus,
    #[serde(rename = "MPESA")]
    Mpesa,
    #[serde(rename = "BLINKBYEMTEL")]
    BlinkByEmtel,
    #[serde(rename = "MCBJUICE")]
    McbJuice,
    /// Scan to Pay (formerly Masterpass)
    #[serde(rename = "MASTERPASS")]
    ScanToPay,
    #[serde(rename = "MAUCAS")]
    Maucas,
    /// 1ForYou / 1Voucher
    #[serde(rename = "1FORYOU")]
    OneForYou,
    #[serde(rename = "MONEYBADGER")]
    MoneyBadger,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PeachApmPaymentType {
    /// Debit (payment)
    DB,
    /// Refund
    RF,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum ApmVirtualAccountType {
    #[serde(rename = "IDNUMBER")]
    IdNumber,
    #[serde(rename = "CELLPHONE")]
    Cellphone,
    #[serde(rename = "ACCOUNTNUMBER")]
    AccountNumber,
}

impl From<common_enums::CapitecPayAccountType> for ApmVirtualAccountType {
    fn from(value: common_enums::CapitecPayAccountType) -> Self {
        match value {
            common_enums::CapitecPayAccountType::IdNumber => Self::IdNumber,
            common_enums::CapitecPayAccountType::Cellphone => Self::Cellphone,
            common_enums::CapitecPayAccountType::AccountNumber => Self::AccountNumber,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum PayShapBank {
    #[serde(rename = "FIRSTNATIONALBANK")]
    FirstNationalBank,
    #[serde(rename = "DISCOVERYBANK")]
    DiscoveryBank,
    #[serde(rename = "NEDBANK")]
    Nedbank,
    #[serde(rename = "TYMEBANK")]
    TymeBank,
    #[serde(rename = "ABSABANK")]
    AbsaBank,
}

impl TryFrom<common_enums::BankNames> for PayShapBank {
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(bank: common_enums::BankNames) -> Result<Self, Self::Error> {
        match bank {
            common_enums::BankNames::FirstNationalBank => Ok(Self::FirstNationalBank),
            common_enums::BankNames::DiscoveryBank => Ok(Self::DiscoveryBank),
            common_enums::BankNames::Nedbank => Ok(Self::Nedbank),
            common_enums::BankNames::TymeBank => Ok(Self::TymeBank),
            common_enums::BankNames::Absa => Ok(Self::AbsaBank),
            _ => Err(errors::ConnectorError::NotSupported {
                message: format!("PayShap with bank {bank:?}"),
                connector: "Peachpaymentsapm",
            }
            .into()),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmVirtualAccount {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<Secret<String>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub account_type: Option<ApmVirtualAccountType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank: Option<PayShapBank>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmBrowser {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept_header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_height: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_width: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_enabled: Option<String>,
    #[serde(rename = "javascriptEnabled", skip_serializing_if = "Option::is_none")]
    pub javascript_enabled: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_color_depth: Option<String>,
}

impl From<&hyperswitch_domain_models::router_request_types::BrowserInformation> for ApmBrowser {
    fn from(
        browser_info: &hyperswitch_domain_models::router_request_types::BrowserInformation,
    ) -> Self {
        Self {
            accept_header: browser_info.accept_header.clone(),
            language: browser_info.language.clone(),
            screen_height: browser_info
                .screen_height
                .map(|screen_height| screen_height.to_string()),
            screen_width: browser_info
                .screen_width
                .map(|screen_width| screen_width.to_string()),
            timezone: browser_info.time_zone.map(|tz| tz.to_string()),
            user_agent: browser_info.user_agent.clone(),
            java_enabled: browser_info
                .java_enabled
                .map(|java_enabled| java_enabled.to_string()),
            javascript_enabled: browser_info
                .java_script_enabled
                .map(|js_enabled| js_enabled.to_string()),
            screen_color_depth: browser_info
                .color_depth
                .map(|color_depth| color_depth.to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmCustomer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<pii::Email>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub given_name: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surname: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merchant_customer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<ApmBrowser>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmAddress {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub street1: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub street2: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postcode: Option<Secret<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<common_enums::CountryAlpha2>,
}

impl ApmAddress {
    fn into_option(self) -> Option<Self> {
        let is_empty = self.street1.is_none()
            && self.street2.is_none()
            && self.city.is_none()
            && self.state.is_none()
            && self.postcode.is_none()
            && self.country.is_none();
        (!is_empty).then_some(self)
    }
}

/// The shopper's RCS store card number is passed in a top level `card` object
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmCard {
    pub number: Secret<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmCartItem {
    pub name: String,
    pub quantity: String,
    pub price: StringMajorUnit,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmCart {
    pub cart_item: Vec<ApmCartItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmCustomParameters {
    pub enable_test_mode: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeachpaymentsApmPaymentsRequest {
    pub authentication: ApmAuthentication,
    pub merchant_transaction_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merchant_invoice_id: Option<String>,
    pub amount: StringMajorUnit,
    pub currency: common_enums::Currency,
    pub payment_brand: PeachPaymentBrand,
    pub payment_type: PeachApmPaymentType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shopper_result_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub virtual_account: Option<ApmVirtualAccount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<ApmCard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer: Option<ApmCustomer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing: Option<ApmAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shipping: Option<ApmAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cart: Option<ApmCart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_parameters: Option<ApmCustomParameters>,
}

/// Billing phone in `+27-711111200` format - the Payments API requires
/// `virtualAccount.accountId` to match `^\+[0-9]{1,3}-[0-9]{1,30}$` for
/// PayShap and M-PESA (sandbox verified)
fn get_hyphenated_international_phone(
    item: &PaymentsAuthorizeRouterData,
) -> Result<Secret<String>, error_stack::Report<errors::ConnectorError>> {
    let phone = item.get_billing_phone()?;
    let country_code = phone.get_country_code()?;
    let number = phone.get_number()?;
    Ok(Secret::new(format!(
        "{}-{}",
        country_code,
        number.peek().trim_start_matches('0')
    )))
}

/// Billing phone in local `0711111200` format - Capitec Pay requires a 10
/// digit cellphone number starting with 0 (sandbox verified)
fn get_local_phone_number(
    item: &PaymentsAuthorizeRouterData,
) -> Result<Secret<String>, error_stack::Report<errors::ConnectorError>> {
    let number = item.get_billing_phone()?.get_number()?;
    Ok(Secret::new(format!(
        "0{}",
        number.peek().trim_start_matches('0')
    )))
}

/// Billing phone in `27711111200` format (digits only, country code without
/// the plus) - required for `customer.mobile` (sandbox verified for 1ForYou)
fn get_customer_mobile(
    item: &PaymentsAuthorizeRouterData,
) -> Result<Secret<String>, error_stack::Report<errors::ConnectorError>> {
    let phone = item.get_billing_phone()?;
    let country_code = phone.extract_country_code()?;
    let number = phone.get_number()?;
    Ok(Secret::new(format!(
        "{}{}",
        country_code,
        number.peek().trim_start_matches('0')
    )))
}

/// M-PESA takes the digits-only number with country code, e.g. `254111111111`
fn digits_cellphone_virtual_account(
    item: &PaymentsAuthorizeRouterData,
) -> Result<ApmVirtualAccount, error_stack::Report<errors::ConnectorError>> {
    Ok(ApmVirtualAccount {
        account_id: Some(get_customer_mobile(item)?),
        password: None,
        account_type: None,
        bank: None,
    })
}

/// blink by Emtel and MCB Juice take the local 8 digit Mauritian number
/// without a country code
fn local_number_virtual_account(
    item: &PaymentsAuthorizeRouterData,
) -> Result<ApmVirtualAccount, error_stack::Report<errors::ConnectorError>> {
    Ok(ApmVirtualAccount {
        account_id: Some(item.get_billing_phone()?.get_number()?),
        password: None,
        account_type: None,
        bank: None,
    })
}

type ApmBrandDetails = (
    PeachPaymentBrand,
    Option<ApmVirtualAccount>,
    Option<ApmCard>,
);

fn get_apm_brand_details(
    item: &PaymentsAuthorizeRouterData,
) -> Result<ApmBrandDetails, error_stack::Report<errors::ConnectorError>> {
    match &item.request.payment_method_data {
        PaymentMethodData::BankTransfer(bank_transfer_data) => match bank_transfer_data.as_ref() {
            BankTransferData::CapitecPay {
                account_type,
                account_id,
            } => {
                let account_id = match account_id {
                    Some(account_id) => account_id.clone(),
                    None => get_local_phone_number(item)?,
                };
                Ok((
                    PeachPaymentBrand::CapitecPay,
                    Some(ApmVirtualAccount {
                        account_id: Some(account_id),
                        password: None,
                        account_type: Some(
                            account_type
                                .map(ApmVirtualAccountType::from)
                                .unwrap_or(ApmVirtualAccountType::Cellphone),
                        ),
                        bank: None,
                    }),
                    None,
                ))
            }
            BankTransferData::PayShap { bank } => {
                let bank = bank.ok_or(errors::ConnectorError::MissingRequiredField {
                    field_name: "payment_method_data.bank_transfer.pay_shap.bank",
                })?;
                Ok((
                    PeachPaymentBrand::PayShap,
                    Some(ApmVirtualAccount {
                        account_id: Some(get_hyphenated_international_phone(item)?),
                        password: None,
                        account_type: Some(ApmVirtualAccountType::Cellphone),
                        bank: Some(PayShapBank::try_from(bank)?),
                    }),
                    None,
                ))
            }
            BankTransferData::NedbankDirectEft {} => {
                Ok((PeachPaymentBrand::NedbankDirectEft, None, None))
            }
            BankTransferData::PeachEft {} => Ok((PeachPaymentBrand::PeachEft, None, None)),
            _ => Err(errors::ConnectorError::NotImplemented(
                utils::get_unimplemented_payment_method_error_message("Peachpaymentsapm"),
            )
            .into()),
        },
        PaymentMethodData::PayLater(pay_later_data) => match pay_later_data {
            PayLaterData::PayflexRedirect {} => Ok((PeachPaymentBrand::Payflex, None, None)),
            PayLaterData::ZeroPayRedirect {} => Ok((PeachPaymentBrand::ZeroPay, None, None)),
            PayLaterData::FloatRedirect {} => Ok((PeachPaymentBrand::Float, None, None)),
            PayLaterData::HappyPayRedirect {} => Ok((PeachPaymentBrand::HappyPay, None, None)),
            PayLaterData::MobicredRedirect { password } => {
                let password =
                    password
                        .clone()
                        .ok_or(errors::ConnectorError::MissingRequiredField {
                            field_name: "payment_method_data.pay_later.mobicred_redirect.password",
                        })?;
                let email = item.get_billing_email()?;
                Ok((
                    PeachPaymentBrand::Mobicred,
                    Some(ApmVirtualAccount {
                        account_id: Some(Secret::new(email.expose().expose())),
                        password: Some(password),
                        account_type: None,
                        bank: None,
                    }),
                    None,
                ))
            }
            // A+ store cards identify the shopper by their account email
            PayLaterData::APlusRedirect {} => {
                let email = item.get_billing_email()?;
                Ok((
                    PeachPaymentBrand::APlus,
                    Some(ApmVirtualAccount {
                        account_id: Some(Secret::new(email.expose().expose())),
                        password: None,
                        account_type: None,
                        bank: None,
                    }),
                    None,
                ))
            }
            // RCS store cards pass the card number in a top level `card` object
            PayLaterData::RcsRedirect { card_number } => Ok((
                PeachPaymentBrand::Rcs,
                None,
                card_number.clone().map(|card_number| ApmCard {
                    number: card_number,
                }),
            )),
            _ => Err(errors::ConnectorError::NotImplemented(
                utils::get_unimplemented_payment_method_error_message("Peachpaymentsapm"),
            )
            .into()),
        },
        PaymentMethodData::Wallet(wallet_data) => match wallet_data {
            WalletData::MpesaRedirect {} => Ok((
                PeachPaymentBrand::Mpesa,
                Some(digits_cellphone_virtual_account(item)?),
                None,
            )),
            WalletData::BlinkByEmtelRedirect {} => Ok((
                PeachPaymentBrand::BlinkByEmtel,
                Some(local_number_virtual_account(item)?),
                None,
            )),
            WalletData::McbJuiceRedirect {} => Ok((
                PeachPaymentBrand::McbJuice,
                Some(local_number_virtual_account(item)?),
                None,
            )),
            WalletData::ScanToPayRedirect {} => Ok((PeachPaymentBrand::ScanToPay, None, None)),
            WalletData::MaucasRedirect {} => Ok((PeachPaymentBrand::Maucas, None, None)),
            _ => Err(errors::ConnectorError::NotImplemented(
                utils::get_unimplemented_payment_method_error_message("Peachpaymentsapm"),
            )
            .into()),
        },
        PaymentMethodData::Voucher(VoucherData::OneForYou(one_for_you_data)) => {
            let voucher_pin = one_for_you_data.voucher_pin.clone().ok_or(
                errors::ConnectorError::MissingRequiredField {
                    field_name: "payment_method_data.voucher.one_for_you.voucher_pin",
                },
            )?;
            Ok((
                PeachPaymentBrand::OneForYou,
                Some(ApmVirtualAccount {
                    account_id: None,
                    password: Some(voucher_pin),
                    account_type: None,
                    bank: None,
                }),
                None,
            ))
        }
        // MoneyBadger is the only crypto method on the Payments API; generic
        // crypto_currency payments are not supported
        PaymentMethodData::Crypto(_)
            if item.request.payment_method_type
                == Some(common_enums::PaymentMethodType::MoneyBadger) =>
        {
            Ok((PeachPaymentBrand::MoneyBadger, None, None))
        }
        _ => Err(errors::ConnectorError::NotImplemented(
            utils::get_unimplemented_payment_method_error_message("Peachpaymentsapm"),
        )
        .into()),
    }
}

impl TryFrom<&PeachpaymentsApmRouterData<&PaymentsAuthorizeRouterData>>
    for PeachpaymentsApmPaymentsRequest
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: &PeachpaymentsApmRouterData<&PaymentsAuthorizeRouterData>,
    ) -> Result<Self, Self::Error> {
        let router_data = item.router_data;
        let authentication = ApmAuthentication::try_from_connector_data(
            &router_data.connector_auth_type,
            &router_data.connector_meta_data,
        )?;
        let (payment_brand, virtual_account, card) = get_apm_brand_details(router_data)?;

        // 1ForYou is a synchronous voucher flow and requires the shopper's mobile number
        let mobile = if payment_brand == PeachPaymentBrand::OneForYou {
            Some(get_customer_mobile(router_data)?)
        } else {
            get_customer_mobile(router_data).ok()
        };
        // Browser details and ip are forwarded when the SDK collects them
        // (confirm: false flows); they are absent on plain server-to-server calls
        let browser_info = router_data.request.browser_info.as_ref();
        let customer = ApmCustomer {
            email: router_data.get_optional_billing_email(),
            given_name: router_data.get_optional_billing_first_name(),
            surname: router_data.get_optional_billing_last_name(),
            mobile,
            ip: browser_info
                .and_then(|browser_info| browser_info.ip_address)
                .map(|ip_address| Secret::new(ip_address.to_string())),
            merchant_customer_id: router_data
                .get_optional_customer_id()
                .map(|customer_id| customer_id.get_string_repr().to_string()),
            browser: browser_info.map(ApmBrowser::from),
        };

        let billing = ApmAddress {
            street1: router_data.get_optional_billing_line1(),
            street2: router_data.get_optional_billing_line2(),
            city: router_data.get_optional_billing_city(),
            state: router_data.get_optional_billing_state(),
            postcode: router_data.get_optional_billing_zip(),
            country: router_data.get_optional_billing_country(),
        }
        .into_option();
        let shipping = ApmAddress {
            street1: router_data.get_optional_shipping_line1(),
            street2: router_data.get_optional_shipping_line2(),
            city: router_data.get_optional_shipping_city(),
            state: router_data.get_optional_shipping_state(),
            postcode: router_data.get_optional_shipping_zip(),
            country: router_data.get_optional_shipping_country(),
        }
        .into_option();

        let cart = router_data
            .request
            .order_details
            .as_ref()
            .map(|order_details| {
                order_details
                    .iter()
                    .map(|order_detail| {
                        Ok(ApmCartItem {
                            name: order_detail.product_name.clone(),
                            quantity: order_detail.quantity.to_string(),
                            price: utils::convert_amount(
                                &StringMajorUnitForConnector,
                                order_detail.amount,
                                router_data.request.currency,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, Self::Error>>()
            })
            .transpose()?
            .map(|cart_item| ApmCart { cart_item });

        let custom_parameters =
            router_data
                .test_mode
                .unwrap_or(false)
                .then(|| ApmCustomParameters {
                    enable_test_mode: "true".to_string(),
                });

        Ok(Self {
            authentication,
            merchant_transaction_id: router_data.connector_request_reference_id.clone(),
            merchant_invoice_id: router_data.request.merchant_order_reference_id.clone(),
            amount: item.amount.clone(),
            currency: router_data.request.currency,
            payment_brand,
            payment_type: PeachApmPaymentType::DB,
            shopper_result_url: router_data.request.router_return_url.clone(),
            virtual_account,
            card,
            customer: Some(customer),
            billing,
            shipping,
            cart,
            custom_parameters,
        })
    }
}

// Payments API responses

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmResult {
    pub code: String,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_errors: Option<Vec<ApmParameterError>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmParameterError {
    pub name: Option<String>,
    pub value: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ApmRedirectMethod {
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "POST")]
    Post,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmRedirectParameter {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmRedirect {
    pub url: String,
    pub method: Option<ApmRedirectMethod>,
    #[serde(default)]
    pub parameters: Vec<ApmRedirectParameter>,
}

impl From<ApmRedirect> for RedirectForm {
    fn from(redirect: ApmRedirect) -> Self {
        Self::Form {
            endpoint: redirect.url,
            method: match redirect.method {
                Some(ApmRedirectMethod::Post) => Method::Post,
                Some(ApmRedirectMethod::Get) | None => Method::Get,
            },
            form_fields: redirect
                .parameters
                .into_iter()
                .map(|parameter| (parameter.name, parameter.value))
                .collect::<HashMap<String, String>>(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeachpaymentsApmPaymentsResponse {
    pub id: String,
    pub result: ApmResult,
    pub redirect: Option<ApmRedirect>,
    pub merchant_transaction_id: Option<String>,
    pub payment_type: Option<PeachApmPaymentType>,
}

/// Maps a Payments API result code to an attempt status.
///
/// * `000.000.*` / `000.100.1*` - transaction succeeded
/// * `000.200.*` - transaction pending (shopper has to complete the redirect)
/// * `100.*` / `200.*` / `800.*` / `900.*` - transaction failed / rejected
pub fn map_apm_result_code_to_attempt_status(code: &str) -> common_enums::AttemptStatus {
    if code.starts_with("000.000.") || code.starts_with("000.100.1") {
        common_enums::AttemptStatus::Charged
    } else if code.starts_with("000.200") {
        common_enums::AttemptStatus::AuthenticationPending
    } else if code.starts_with("100.")
        || code.starts_with("200.")
        || code.starts_with("800.")
        || code.starts_with("900.")
    {
        common_enums::AttemptStatus::Failure
    } else {
        common_enums::AttemptStatus::Pending
    }
}

pub fn map_apm_result_code_to_refund_status(code: &str) -> common_enums::RefundStatus {
    if code.starts_with("000.000.") || code.starts_with("000.100.1") {
        common_enums::RefundStatus::Success
    } else if code.starts_with("100.")
        || code.starts_with("200.")
        || code.starts_with("800.")
        || code.starts_with("900.")
    {
        common_enums::RefundStatus::Failure
    } else {
        common_enums::RefundStatus::Pending
    }
}

pub fn build_apm_error_reason(result: &ApmResult) -> Option<String> {
    let parameter_errors = result.parameter_errors.as_ref().map(|parameter_errors| {
        parameter_errors
            .iter()
            .map(|parameter_error| {
                format!(
                    "{}: {}",
                    parameter_error.name.as_deref().unwrap_or("parameter"),
                    parameter_error.message.as_deref().unwrap_or("invalid")
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    });
    match (result.description.clone(), parameter_errors) {
        (Some(description), Some(parameter_errors)) => {
            Some(format!("{description} - {parameter_errors}"))
        }
        (Some(description), None) => Some(description),
        (None, parameter_errors) => parameter_errors,
    }
}

impl<F, T> TryFrom<ResponseRouterData<F, PeachpaymentsApmPaymentsResponse, T, PaymentsResponseData>>
    for RouterData<F, T, PaymentsResponseData>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: ResponseRouterData<F, PeachpaymentsApmPaymentsResponse, T, PaymentsResponseData>,
    ) -> Result<Self, Self::Error> {
        let status = map_apm_result_code_to_attempt_status(&item.response.result.code);
        let response = if utils::is_payment_failure(status) {
            Err(ErrorResponse {
                code: item.response.result.code.clone(),
                message: item
                    .response
                    .result
                    .description
                    .clone()
                    .unwrap_or(NO_ERROR_MESSAGE.to_string()),
                reason: build_apm_error_reason(&item.response.result),
                status_code: item.http_code,
                attempt_status: Some(status),
                connector_transaction_id: Some(item.response.id.clone()),
                connector_response_reference_id: item.response.merchant_transaction_id.clone(),
                network_advice_code: None,
                network_decline_code: None,
                network_error_message: None,
                connector_metadata: None,
            })
        } else {
            Ok(PaymentsResponseData::TransactionResponse {
                resource_id: ResponseId::ConnectorTransactionId(item.response.id.clone()),
                redirection_data: Box::new(item.response.redirect.map(RedirectForm::from)),
                mandate_reference: Box::new(None),
                connector_metadata: None,
                network_txn_id: None,
                network_txn_link_id: None,
                connector_response_reference_id: item.response.merchant_transaction_id.clone(),
                incremental_authorization_allowed: None,
                authentication_data: None,
                charges: None,
            })
        };

        Ok(Self {
            status,
            response,
            ..item.data
        })
    }
}

// Payments API refunds

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeachpaymentsApmRefundRequest {
    pub authentication: ApmAuthentication,
    pub amount: StringMajorUnit,
    pub currency: common_enums::Currency,
    pub payment_type: PeachApmPaymentType,
}

impl TryFrom<&PeachpaymentsApmRouterData<&RefundsRouterData<Execute>>>
    for PeachpaymentsApmRefundRequest
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: &PeachpaymentsApmRouterData<&RefundsRouterData<Execute>>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            authentication: ApmAuthentication::try_from_connector_data(
                &item.router_data.connector_auth_type,
                &item.router_data.connector_meta_data,
            )?,
            amount: item.amount.clone(),
            currency: item.router_data.request.currency,
            payment_type: PeachApmPaymentType::RF,
        })
    }
}

fn get_apm_refund_response(
    response: &PeachpaymentsApmPaymentsResponse,
    http_code: u16,
) -> Result<RefundsResponseData, Box<ErrorResponse>> {
    let refund_status = map_apm_result_code_to_refund_status(&response.result.code);
    if refund_status == common_enums::RefundStatus::Failure {
        Err(Box::new(ErrorResponse {
            code: response.result.code.clone(),
            message: response
                .result
                .description
                .clone()
                .unwrap_or(NO_ERROR_MESSAGE.to_string()),
            reason: build_apm_error_reason(&response.result),
            status_code: http_code,
            attempt_status: None,
            connector_transaction_id: None,
            connector_response_reference_id: response.merchant_transaction_id.clone(),
            network_advice_code: None,
            network_decline_code: None,
            network_error_message: None,
            connector_metadata: None,
        }))
    } else {
        Ok(RefundsResponseData {
            connector_refund_id: response.id.clone(),
            refund_status,
        })
    }
}

impl
    TryFrom<
        ResponseRouterData<
            Execute,
            PeachpaymentsApmPaymentsResponse,
            RefundsData,
            RefundsResponseData,
        >,
    > for RefundsRouterData<Execute>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: ResponseRouterData<
            Execute,
            PeachpaymentsApmPaymentsResponse,
            RefundsData,
            RefundsResponseData,
        >,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            response: get_apm_refund_response(&item.response, item.http_code)
                .map_err(|error_response| *error_response),
            ..item.data
        })
    }
}

impl
    TryFrom<
        ResponseRouterData<
            RSync,
            PeachpaymentsApmPaymentsResponse,
            RefundsData,
            RefundsResponseData,
        >,
    > for RefundsRouterData<RSync>
{
    type Error = error_stack::Report<errors::ConnectorError>;
    fn try_from(
        item: ResponseRouterData<
            RSync,
            PeachpaymentsApmPaymentsResponse,
            RefundsData,
            RefundsResponseData,
        >,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            response: get_apm_refund_response(&item.response, item.http_code)
                .map_err(|error_response| *error_response),
            ..item.data
        })
    }
}

// Payments API webhooks (decrypted payload)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeachpaymentsApmWebhook {
    pub id: String,
    pub payment_type: Option<PeachApmPaymentType>,
    pub payment_brand: Option<String>,
    pub merchant_transaction_id: Option<String>,
    pub currency: Option<String>,
    pub result: ApmResult,
    pub timestamp: Option<String>,
}

/// Some Payments API webhooks wrap the hex-encoded ciphertext in a JSON object
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApmEncryptedWebhookBody {
    pub encrypted_body: String,
}

// Payments API error response

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeachpaymentsApmErrorResponse {
    pub result: ApmResult,
    pub id: Option<String>,
    pub merchant_transaction_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payment_brand_serialization() {
        let cases = [
            (PeachPaymentBrand::CapitecPay, "\"CAPITECPAY\""),
            (PeachPaymentBrand::PayShap, "\"PAYSHAP\""),
            (PeachPaymentBrand::NedbankDirectEft, "\"NEDBANKDIRECTEFT\""),
            (PeachPaymentBrand::PeachEft, "\"PEACHEFT\""),
            (PeachPaymentBrand::Payflex, "\"PAYFLEX\""),
            (PeachPaymentBrand::ZeroPay, "\"ZEROPAY\""),
            (PeachPaymentBrand::Float, "\"FLOAT\""),
            (PeachPaymentBrand::HappyPay, "\"HAPPYPAY\""),
            (PeachPaymentBrand::Mobicred, "\"MOBICRED\""),
            (PeachPaymentBrand::Rcs, "\"RCS\""),
            (PeachPaymentBrand::APlus, "\"APLUS\""),
            (PeachPaymentBrand::Mpesa, "\"MPESA\""),
            (PeachPaymentBrand::BlinkByEmtel, "\"BLINKBYEMTEL\""),
            (PeachPaymentBrand::McbJuice, "\"MCBJUICE\""),
            (PeachPaymentBrand::ScanToPay, "\"MASTERPASS\""),
            (PeachPaymentBrand::Maucas, "\"MAUCAS\""),
            (PeachPaymentBrand::OneForYou, "\"1FORYOU\""),
            (PeachPaymentBrand::MoneyBadger, "\"MONEYBADGER\""),
        ];
        for (brand, expected) in cases {
            assert_eq!(
                serde_json::to_string(&brand).expect("brand serialization failed"),
                expected
            );
        }
    }

    #[test]
    fn test_apm_result_code_to_attempt_status() {
        assert_eq!(
            map_apm_result_code_to_attempt_status("000.000.000"),
            common_enums::AttemptStatus::Charged
        );
        assert_eq!(
            map_apm_result_code_to_attempt_status("000.100.110"),
            common_enums::AttemptStatus::Charged
        );
        assert_eq!(
            map_apm_result_code_to_attempt_status("000.200.000"),
            common_enums::AttemptStatus::AuthenticationPending
        );
        // user cancelled
        assert_eq!(
            map_apm_result_code_to_attempt_status("100.396.101"),
            common_enums::AttemptStatus::Failure
        );
        // declined
        assert_eq!(
            map_apm_result_code_to_attempt_status("800.100.152"),
            common_enums::AttemptStatus::Failure
        );
        // request validation error
        assert_eq!(
            map_apm_result_code_to_attempt_status("800.400.100"),
            common_enums::AttemptStatus::Failure
        );
        assert_eq!(
            map_apm_result_code_to_attempt_status("900.100.300"),
            common_enums::AttemptStatus::Failure
        );
        assert_eq!(
            map_apm_result_code_to_attempt_status("000.400.000"),
            common_enums::AttemptStatus::Pending
        );
    }

    #[test]
    fn test_apm_result_code_to_refund_status() {
        assert_eq!(
            map_apm_result_code_to_refund_status("000.000.000"),
            common_enums::RefundStatus::Success
        );
        assert_eq!(
            map_apm_result_code_to_refund_status("000.200.000"),
            common_enums::RefundStatus::Pending
        );
        assert_eq!(
            map_apm_result_code_to_refund_status("800.100.152"),
            common_enums::RefundStatus::Failure
        );
    }

    #[test]
    fn test_apm_redirect_to_redirect_form() {
        let redirect = ApmRedirect {
            url: "https://example.peachpayments.com/redirect".to_string(),
            method: Some(ApmRedirectMethod::Post),
            parameters: vec![ApmRedirectParameter {
                name: "token".to_string(),
                value: "abc123".to_string(),
            }],
        };
        match RedirectForm::from(redirect) {
            RedirectForm::Form {
                endpoint,
                method,
                form_fields,
            } => {
                assert_eq!(endpoint, "https://example.peachpayments.com/redirect");
                assert_eq!(method, Method::Post);
                assert_eq!(form_fields.get("token"), Some(&"abc123".to_string()));
            }
            _ => panic!("expected RedirectForm::Form"),
        }
    }

    #[test]
    fn test_a_plus_serde_names() {
        // payment_method_type wire name
        assert_eq!(
            serde_json::to_string(&common_enums::PaymentMethodType::APlus)
                .expect("pmt serialization failed"),
            "\"a_plus\""
        );
        // payment_method_data key
        let data = hyperswitch_domain_models::payment_method_data::PayLaterData::APlusRedirect {};
        assert_eq!(
            serde_json::to_string(&data).expect("data serialization failed"),
            "{\"APlusRedirect\":{}}"
        );
    }

    #[test]
    fn test_apm_browser_from_browser_information() {
        let browser_info = hyperswitch_domain_models::router_request_types::BrowserInformation {
            color_depth: Some(24),
            java_enabled: Some(false),
            java_script_enabled: Some(true),
            language: Some("EN".to_string()),
            screen_height: Some(1080),
            screen_width: Some(1920),
            time_zone: Some(30),
            ip_address: None,
            accept_header: Some("application/json".to_string()),
            user_agent: Some("Mozilla/5.0".to_string()),
            os_type: None,
            os_version: None,
            device_model: None,
            accept_language: None,
            referer: None,
        };
        let browser = ApmBrowser::from(&browser_info);
        let json = serde_json::to_value(&browser).expect("browser serialization failed");
        assert_eq!(json["screenHeight"], "1080");
        assert_eq!(json["screenWidth"], "1920");
        assert_eq!(json["javascriptEnabled"], "true");
        assert_eq!(json["javaEnabled"], "false");
        assert_eq!(json["screenColorDepth"], "24");
        assert_eq!(json["timezone"], "30");
    }

    #[test]
    fn test_apm_payments_response_deserialization() {
        let body = r#"{
            "id": "8ac7a4a09c8b9c8e019c8ba47c0000aa",
            "result": { "code": "000.200.000", "description": "transaction pending" },
            "redirect": {
                "url": "https://redirect.example.com",
                "method": "GET",
                "parameters": []
            },
            "merchantTransactionId": "Abc123Def456Gh78"
        }"#;
        let response: PeachpaymentsApmPaymentsResponse =
            serde_json::from_str(body).expect("response deserialization failed");
        assert_eq!(response.id, "8ac7a4a09c8b9c8e019c8ba47c0000aa");
        assert_eq!(response.result.code, "000.200.000");
        assert_eq!(
            response.merchant_transaction_id.as_deref(),
            Some("Abc123Def456Gh78")
        );
        assert_eq!(
            response.redirect.expect("redirect missing").method,
            Some(ApmRedirectMethod::Get)
        );
    }
}
