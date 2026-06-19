import { getCurrency, getCustomExchange } from "./Modifiers";

// IMPORTANT: Paydunya requires a store name on every invoice, surfaced from
// the connector account's `metadata.store_name` (config field is
// `required = true`). The merchant connector account in these tests is
// provisioned from your `creds.json`, where metadata is merged in from the
// `paydunya` entry. That entry MUST include `metadata.store_name`, e.g.:
//
//   "paydunya": {
//     "connector_account_details": {
//       "auth_type": "...",
//       "api_key": "...",
//       "key1": "..."
//     },
//     "metadata": { "store_name": "Your Store" }
//   }
//
// Without it, the order-create leg fails with `InvalidConnectorConfig`
// (`metadata.store_name`) and every wallet flow below errors instead of
// returning the asserted `processing` response.

// Paydunya only exposes mobile-money rails today (SOFTPAY). Each rail maps to
// a dedicated Hyperswitch wallet payment method type — `momo` (MTN family),
// `moov_money` (Moov family), `wave`, `orange_money`, `djamo`, `t_money`,
// `wizall`, `expresso` and `free_money` — and the operator endpoint is resolved
// from
// `(payment_method_type, billing.country)` (see `PaydunyaOperator`). Card
// rails, refunds, voids, captures and mandate flows are `NotImplemented`
// upstream and intentionally fall back to the default "not implemented"
// responses defined in `Commons.js`.
//
// Every SOFTPAY rail carries a dedicated wallet-data variant: `momo`
// (MomoRedirect), `moov_money` (MoovMoneyRedirect), `wave` (WaveRedirect),
// `orange_money` (OrangeMoneyRedirect), `djamo` (DjamoRedirect), `t_money`
// (TMoneyRedirect), `wizall` (WizallRedirect), `expresso`
// (ExpressoRedirect) and `free_money` (FreeMoneyRedirect).

// SOFTPAY requires the payer's full name, phone number and email — without
// them the connector returns `MissingRequiredField`. Country is what drives
// the operator resolution (BJ -> MTN Benin, SN -> Orange
// Money/Wizall/Expresso/Free Money Senegal, CI -> Djamo Côte d'Ivoire, TG ->
// T-Money Togo, etc.).
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

// Moov Benin — payment_method_type=moov_money + country=BJ resolves to the
// `softpay/moov-benin` endpoint.
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

// Wave Senegal — payment_method_type=wave + country=SN resolves to the
// `softpay/wave-senegal` endpoint.
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

// Orange Money Senegal — country=SN resolves to the
// `softpay/new-orange-money-senegal` endpoint (no payer OTP required).
const orangeMoneySenegalBilling = {
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

// Djamo Côte d'Ivoire — country=CI resolves to the shared `softpay/djamo`
// endpoint with `code_country=ci`.
const djamoCiBilling = {
  address: {
    line1: "Boulevard Latrille",
    line2: "Abidjan",
    city: "Abidjan",
    state: "Abidjan",
    zip: "00225",
    country: "CI",
    first_name: "Camille",
    last_name: "Coulibaly",
  },
  phone: {
    number: "0777568646",
    country_code: "+225",
  },
  email: "camille.coulibaly@example.com",
};

// T-Money Togo — Togo is the only region Paydunya exposes for T-Money, so any
// T-Money attempt resolves to the single `softpay/t-money-togo` endpoint.
const tmoneyTogoBilling = {
  address: {
    line1: "Boulevard du Mono",
    line2: "Lome",
    city: "Lome",
    state: "Maritime",
    zip: "00228",
    country: "TG",
    first_name: "Kofi",
    last_name: "Mensah",
  },
  phone: {
    number: "70707070",
    country_code: "+228",
  },
  email: "kofi.mensah@example.com",
};

// Wizall Money is Senegal-only: payment_method_type=wizall resolves to the
// `softpay/wizall-senegal` endpoint. Billing country (SN) and the payer's
// full name / phone / email are required, mirroring the Wave Senegal rail.
const wizallSenegalBilling = {
  address: {
    line1: "Rue Carnot",
    line2: "Dakar",
    city: "Dakar",
    state: "Dakar",
    zip: "10000",
    country: "SN",
    first_name: "Fatou",
    last_name: "Sow",
  },
  phone: {
    number: "780000000",
    country_code: "+221",
  },
  email: "fatou.sow@example.com",
};

// Expresso is Senegal-only: payment_method_type=expresso resolves to the
// `softpay/expresso-senegal` endpoint. Billing country (SN) and the payer's
// full name / phone / email are required, mirroring the Wave Senegal rail.
const expressoSenegalBilling = {
  address: {
    line1: "Avenue Bourguiba",
    line2: "Dakar",
    city: "Dakar",
    state: "Dakar",
    zip: "10000",
    country: "SN",
    first_name: "Moussa",
    last_name: "Diop",
  },
  phone: {
    number: "760000000",
    country_code: "+221",
  },
  email: "moussa.diop@example.com",
};

// Free Money is Senegal-only: payment_method_type=free_money resolves to the
// `softpay/free-money-senegal` endpoint. Billing country (SN) and the payer's
// full name / phone / email are required, mirroring the Wave Senegal rail.
const freeMoneySenegalBilling = {
  address: {
    line1: "Rue Carnot",
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
const PAYDUNYA_WALLET_TYPES = new Set([
  "Momo",
  "MoovMoney",
  "Wave",
  "OrangeMoney",
  "Djamo",
  "TMoney",
  "Wizall",
  "Expresso",
  "FreeMoney",
]);

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
    // Moov Benin — payment_method_type=moov_money + country=BJ resolves to the
    // `softpay/moov-benin` endpoint.
    MoovMoney: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "moov_money",
        payment_method_data: {
          wallet: {
            moov_money_redirect: {},
          },
        },
        currency: "XOF",
        billing: moovBeninBilling,
        email: moovBeninBilling.email,
      },
      Response: softpayPendingResponse,
    }),
    // Wave Senegal — payment_method_type=wave + country=SN resolves to the
    // `softpay/wave-senegal` endpoint.
    Wave: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "wave",
        payment_method_data: {
          wallet: {
            wave_redirect: {},
          },
        },
        currency: "XOF",
        billing: waveSenegalBilling,
        email: waveSenegalBilling.email,
      },
      Response: softpayPendingResponse,
    }),
    // Orange Money Senegal — payment_method_type=orange_money + country=SN
    // resolves to the `softpay/new-orange-money-senegal` endpoint.
    OrangeMoney: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "orange_money",
        payment_method_data: {
          wallet: {
            orange_money_redirect: {},
          },
        },
        currency: "XOF",
        billing: orangeMoneySenegalBilling,
        email: orangeMoneySenegalBilling.email,
      },
      Response: softpayPendingResponse,
    }),
    // Djamo Côte d'Ivoire — payment_method_type=djamo + country=CI resolves to
    // the shared `softpay/djamo` endpoint with `code_country=ci`.
    Djamo: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "djamo",
        payment_method_data: {
          wallet: {
            djamo_redirect: {},
          },
        },
        currency: "XOF",
        billing: djamoCiBilling,
        email: djamoCiBilling.email,
      },
      Response: softpayPendingResponse,
    }),
    // T-Money Togo — payment_method_type=t_money resolves to the single
    // `softpay/t-money-togo` endpoint regardless of billing country.
    TMoney: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "t_money",
        payment_method_data: {
          wallet: {
            t_money_redirect: {},
          },
        },
        currency: "XOF",
        billing: tmoneyTogoBilling,
        email: tmoneyTogoBilling.email,
      },
      Response: softpayPendingResponse,
    }),
    // Wizall Senegal — payment_method_type=wizall + country=SN resolves to
    // the `softpay/wizall-senegal` endpoint.
    Wizall: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "wizall",
        payment_method_data: {
          wallet: {
            wizall_redirect: {},
          },
        },
        currency: "XOF",
        billing: wizallSenegalBilling,
        email: wizallSenegalBilling.email,
      },
      Response: softpayPendingResponse,
    }),
    // Expresso Senegal — payment_method_type=expresso + country=SN resolves to
    // the `softpay/expresso-senegal` endpoint.
    Expresso: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "expresso",
        payment_method_data: {
          wallet: {
            expresso_redirect: {},
          },
        },
        currency: "XOF",
        billing: expressoSenegalBilling,
        email: expressoSenegalBilling.email,
      },
      Response: softpayPendingResponse,
    }),
    // Free Money Senegal — payment_method_type=free_money + country=SN resolves
    // to the `softpay/free-money-senegal` endpoint.
    FreeMoney: getCustomExchange({
      Request: {
        payment_method: "wallet",
        payment_method_type: "free_money",
        payment_method_data: {
          wallet: {
            free_money_redirect: {},
          },
        },
        currency: "XOF",
        billing: freeMoneySenegalBilling,
        email: freeMoneySenegalBilling.email,
      },
      Response: softpayPendingResponse,
    }),
  },
};
