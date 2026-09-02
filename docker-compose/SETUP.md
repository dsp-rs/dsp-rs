# Demo setup

## Start services

```sh
docker-compose up
```

## Create a wallet for each of the participants

NOTE: this step has to be performed only once, wallet data is persisted under `[party-a|party-b]/wallet/data`.

### Create a keypair

```sh
cargo run --bin keygen --features keygen
```

Update the private key PEM inside the connector configurations: `[party-a|party-b]/connector/config.json`.

### Setup the wallet

```sh
# create a wallet
WALLET_ID=$(curl -s -X POST http://localhost:17005/wallet \
  -H "Content-Type: application/json" \
  -d '{}' | jq -r '.walletId')
echo $WALLET_ID

# import generated private key
KEY_ID=$(curl -s -X POST http://localhost:17005/wallet/$WALLET_ID/keys/import \
  -H "Content-Type: application/json" \
  -d '{
    "key": {
      "type": "jwk",
      "jwk": {
        "crv": "P-256",
        "d": "L01jaTa977ngpyVYpFI_gRJJtAPZedNk1l_2zWlVRCc",
        "kty": "EC",
        "x": "R0vym-VxjtpMZArz-fpFhjkRkrNOKWu_GrrMWXr8Uks",
        "y": "qCQInpBSrA1b9-ckq-Dzeyk3v2_4JnlDAgQ9O__fv1w"
      }
    }
  }' | jq -r '.keyId')
echo $KEY_ID

# create a DID
HOLDER_DID=$(curl -s -X POST http://localhost:17005/wallet/$WALLET_ID/dids/create \
  -H "Content-Type: application/json" \
  -d '{
    "method": "web",
    "keyId": "$KEY_ID",
    "options": {
      "domain": "party-a-connector:3000",
      "path": ""
    }
  }' | jq -r '.did')
echo $HOLDER_DID
```

NOTE: for `party-b` use the address `http://localhost:27005` and domain `party-b-connector:3000`. The connector provides
the endpoint `/.well-known/did.json`, from which the DID will be resolved.

Update `[party-a|party-b]/connector/config.json` with the respective IDs of the newly created wallets. After that,
restart the services such that the connectors are aware of the correct wallet IDs.

```sh
docker-compose down
docker-compose up
```

## Claim credentials from issuer

For each participant, claim a credential from the issuer.

```sh
# create a credential offer
OFFER_URL=$(curl -s -X POST 'http://localhost:7002/issuer2/credential-offers' \
  -H 'Content-Type: application/json' \
  -d '{
    "profileId": "identityCredentialSdJwt",
    "authMethod": "PRE_AUTHORIZED",
    "runtimeOverrides": {
      "credentialData": {
        "given_name": "Albert",
        "family_name": "Einstein",
        "email": "a.einstein@princeton.edu",
        "phone_number": "+49301234567",
        "address": {
          "street_address": "Einsteinstrasse 1",
          "locality": "Potsdam",
          "region": "Brandenburg",
          "country": "DE"
        },
        "birthdate": "1879-03-14",
        "is_over_18": true,
        "is_over_21": true,
        "is_over_65": true
      }
    }
  }' | jq -r '.credentialOffer')
echo "OFFER_URL=\"$OFFER_URL\""

# claim offer 
WALLET_ID="..."
curl -s -X POST http://localhost:17005/wallet/$WALLET_ID/credentials/receive \
  -H "Content-Type: application/json" \
  -d "{\"offerUrl\":\"$OFFER_URL\"}"
```

NOTE: for `party-b` use port `27005`.
NOTE: make sure to use the correct wallet IDs for `party-a` and `party-b`.
