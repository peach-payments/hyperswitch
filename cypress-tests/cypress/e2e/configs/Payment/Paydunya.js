import { getCurrency, getCustomExchange } from "./Modifiers";

// Paydunya only exposes mobile-money rails today (SOFTPAY: MTN, Moov, Wave,
// Orange Money, Free Money, Expresso). It maps to Hyperswitch wallet payment
// method types: `momo` (MTN family), `mobile_pay` (Moov family) and `mb_way`
// (Wave family). Card rails, refunds, voids, captures and mandate flows are
// `NotImplemented` upstream and intentionally fall back to the default
// "not implemented" responses defined in `Commons.js`.

// SOFTPAY requires the payer's full name, phone number and email — without
// them the connector returns `MissingRequiredField`. Country is what drives
// the operator resolution (BJ -> MTN/Moov Benin, SN -> Wave Senegal, etc.).
const mtnBeninBilling = {
  address: {
    line1: "Rue 12.345",
    line2: "Cotonou",
    city: "Cotonou",
    state: "Littoral",
    zip: "00229",
    country: "BJ",
    first_name: "Kossi",
    last_name: "Ahouanou",
  },
  phone: {
    number: "90000000",
    country_code: "+229",
  },
  email: "kossi.ahouanou@example.com",
};

const moovBeninBilling = {
  address: {
    line1: "Boulevard Saint-Michel",
    line2: "Cotonou",
    city: "Cotonou",
    state: "Littoral",
    zip: "00229",
    country: "BJ",
    first_name: "Adjoa",
    last_name: "Hounsou",
  },
  phone: {
    number: "60000000",
    country_code: "+229",
  },
  email: "adjoa.hounsou@example.com",
};

const waveSenegalBilling = {
  address: {
    line1: "Avenue Cheikh Anta Diop",
    line2: "Dakar",
    city: "Dakar",
    state: "Dakar",
    zip: "10000",
    country: "SN",
    first_name: "Awa",
    last_name: "Ndiaye",
  },
  phone: {
    number: "770000000",
    country_code: "+221",
  },
  email: "awa.ndiaye@example.com",
};

// Paydunya operates in XOF (UEMOA) and XAF (CEMAC). Hyperswitch accepts both
// as zero-decimal currencies, so amounts are passed in the smallest unit
// (1500 = 1,500 XOF / XAF). For Paydunya's own mobile-money rails we force
// XOF, but the wallet spec is shared with `Bluecode`, `AliPayHk` etc. where
// the connector-agnostic currency map in `Modifiers.getCurrency` is correct;
// defer to it when the spec asks for a non-Paydunya payment method type.
const PAYDUNYA_WALLET_TYPES = new Set(["Momo", "MobilePay", "MbWay"]);

const paydunyaPaymentIntent = (paymentMethodType) => {
  const currency = PAYDUNYA_WALLET_TYPES.has(paymentMethodType)
    ? "XOF"
    : getCurrency(paymentMethodType);
  return getCustomExchange({
    Request: {
      currency,
    },
    Response: {
      status: 200,
      body: {
        status: "requires_payment_method",
      },
    },
  });
};

// The SOFTPAY confirmation returns `processing` while the payer is asked to
// validate the transaction on their mobile (push notification / USSD). The
// IPN webhook ultimately drives the payment to `succeeded`, but the initial
// `/payments/confirm` call surfaces a pending intent — we assert on that
// here rather than coupling the test to webhook delivery timing.
const softpayPendingResponse = {
  status: 200,
  body: {
    status: "processing",
  },
};

export const connectorDetails = {
  card_pm: {
    // Paydunya rejects card mandates inside `validate_mandate_payment`, so any
    // setup-mandate flow is guaranteed to fail. Pin the response so the
    // ZeroAuth spec doesn't fall over.
    ZeroAuthMandate: {
      Response: {
        status: 501,
        body: {
          error: {
            type: "invalid_request",
            message: "Setup Mandate flow for Paydunya is not implemented",
            code: "IR_00",
          },
        },
      },
    },
    ZeroAuthPaymentIntent: {
      Request: {
        amount: 0,
        setup_future_usage: "off_session",
        currency: "XOF",
      },
      Response: {
        status: 200,
        body: {
          status: "requires_payment_method",
          setup_future_usage: "off_session",
        },
      },
    },
    ZeroAuthConfirmPayment: {
      Request: {
        payment_type: "setup_mandate",
        payment_method: "card",
        payment_method_type: "credit",
        payment_method_data: {
          card: {
            card_number: "4111111111111111",
            card_exp_month: "03",
            card_exp_year: "30",
            card_holder_name: "John Doe",
            card_cvc: "737",
          },
        },
      },
      Response: {
        status: 501,
        body: {
          error: {
            type: "invalid_request",
            message: "Setup Mandate flow for Paydunya is not implemented",
            code: "IR_00",
          },
        },
      },
    },
  },
  wallet_pm: {
    PaymentIntent: (paymentMethodType) =>
      paydunyaPaymentIntent(paymentMethodType),
    // MTN Benin — payment_method_type=momo + country=BJ resolves to the
    // `softpay/mtn-benin` endpoint with `mtn_benin_wallet_provider=MTNBENIN`.
    Momo: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "momo",
        payment_method_data: {
          wallet: {
            momo_redirect: {},
          },
        },
        currency: "XOF",
        billing: mtnBeninBilling,
        email: mtnBeninBilling.email,
      },
      Response: softpayPendingResponse,
    }),
    // Moov Benin — payment_method_type=mobile_pay + country=BJ resolves to
    // the `softpay/moov-benin` endpoint.
    MobilePay: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "mobile_pay",
        payment_method_data: {
          wallet: {
            mobile_pay_redirect: {},
          },
        },
        currency: "XOF",
        billing: moovBeninBilling,
        email: moovBeninBilling.email,
      },
      Response: softpayPendingResponse,
    }),
    // Wave Senegal — payment_method_type=mb_way + country=SN resolves to
    // the `softpay/wave-senegal` endpoint.
    MbWay: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "mb_way",
        payment_method_data: {
          wallet: {
            mb_way_redirect: {},
          },
        },
        currency: "XOF",
        billing: waveSenegalBilling,
        email: waveSenegalBilling.email,
      },
      Response: softpayPendingResponse,
    }),
  },
};
