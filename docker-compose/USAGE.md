# Using the connectors to negotiate contracts and transfer data assets

## Catalog sync

The connector for `party-a` provides a demo dataset, which can be found under `party-a/data/datasets`. Upon syncing
the catalogs, `party-b` will discover and download the demo dataset and will persist it under
`party-b/data/datasets/federated/party-a`.

## Negotiating a contract

`party-b` is assumed to be the consumer, which starts a negotiation for the discovered dataset of `party-a`.

```sh
# start negotiation
NEGOTIATION_CONSUMER_PID=$(curl -s -H "Content-Type: application/json" http://localhost:23000/api-internal/negotiate \
  -d '{
    "name": "party-a", 
    "dataset_id": "urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57",
    "offer_id": "urn:uuid:2828282:3dd1add8-4d2d-569e-d634-8394a8836a88"
  }' | jq -r '.consumer_pid')
echo "$NEGOTIATION_CONSUMER_PID"

# check status
AGREEMENT_ID=$(curl -s http://localhost:23000/api-internal/negotiate/$NEGOTIATION_CONSUMER_PID \
  | jq -r '.state | select(.type == "finalized") | .agreement["@id"]')
echo "$AGREEMENT_ID"
```

Note: you can check the full agreement by omiting the `jq` filter above:

```json
{
  "type": "consumer",
  "provider_pid": "urn:uuid:2f75b71d-276e-431f-97dd-ca46223f3819",
  "consumer_pid": "urn:uuid:fa150d2f-eb8d-434f-bd7a-91e3960ddc48",
  "state": {
    "type": "finalized",
    "agreement": {
      "@type": "Agreement",
      "@id": "urn:uuid:6f8fd102-11bc-408f-b489-52fe6353c5f2",
      "permission": [
        {
          "action": "use",
          "constraint": [
            {
              "leftOperand": "spatial",
              "operator": "isPartOf",
              "rightOperand": "_:EU"
            }
          ]
        }
      ],
      "target": "urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57",
      "assigner": "did:web:party-a-connector%3A3000",
      "assignee": "did:web:party-b-connector%3A3000",
      "timestamp": "2026-08-31T11:06:23Z"
    }
  },
  "connector": {
    "connectorAddress": "http://party-a-connector:3000",
    "providerId": "party-a"
  }
}
```

## Start a transfer

```sh
# start transfer
TRANSFER_CONSUMER_PID=$(curl -s -H "Content-Type: application/json" http://localhost:23000/api-internal/transfer \
  -d '{
    "agreement_id": "'$AGREEMENT_ID'",
    "format": "HttpData-PULL"
  }' | jq -r '.consumer_pid')
echo "$TRANSFER_CONSUMER_PID"

# check status
curl -s http://localhost:23000/api-internal/transfer/$TRANSFER_CONSUMER_PID
```

Upon successful transfer start, the status should contain the endpoint and token for accessing the dataset:

```json
{
  "type": "consumer",
  "process": {
    "provider_pid": "urn:uuid:1536e189-df94-4e3b-8ff5-6544434dfad7",
    "consumer_pid": "urn:uuid:96fc6049-22e1-469b-aadc-31349f331f65",
    "state": {
      "type": "started",
      "sent": true
    }
  },
  "agreement": {
    "@type": "Agreement",
    "@id": "urn:uuid:6f8fd102-11bc-408f-b489-52fe6353c5f2",
    "permission": [
      {
        "action": "use",
        "constraint": [
          {
            "leftOperand": "spatial",
            "operator": "isPartOf",
            "rightOperand": "_:EU"
          }
        ]
      }
    ],
    "target": "urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57",
    "assigner": "did:web:party-a-connector%3A3000",
    "assignee": "did:web:party-b-connector%3A3000",
    "timestamp": "2026-08-31T11:06:23Z"
  },
  "format": "HttpData-PULL",
  "connector": {
    "connectorAddress": "http://party-a-connector:3000",
    "providerId": "party-a"
  },
  "data_address": {
    "@type": "DataAddress",
    "endpointType": "https://w3id.org/idsa/v4.1/HTTP",
    "endpoint": "http://party-a-connector:3000/pull",
    "endpointProperties": [
      {
        "@type": "EndpointProperty",
        "name": "authorization",
        "value": "eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ1cm46dXVpZDoxNTM2ZTE4OS1kZjk0LTRlM2ItOGZmNS02NTQ0NDM0ZGZhZDciLCJpc3MiOiJkaWQ6d2ViOnBhcnR5LWEtY29ubmVjdG9yJTNBMzAwMCIsImp0aSI6IjQyMDcwOWE4LTYxMzMtNDhiZi1iM2RmLTMyZGI1NDc3M2RiNSIsImlhdCI6MTc4ODE3NDkzNCwiZXhwIjoxNzg4MjAzNzM0fQ.DV9ceG85wYc1KJByPOGJVI_5xhybS2bevrj6lCBoCV0Pe3CgHGY-XU17PAWwf_vCrlmYJrZ-x8zMvxB0B0S0ZA"
      },
      {
        "@type": "EndpointProperty",
        "name": "authType",
        "value": "bearer"
      }
    ]
  }
}
```

## Access dataset/service

NOTE: we have to map the returned endpoint address of `party-a` to localhost:
`http://party-a-connector:3000/pull --> http://localhost:13000/pull`

```sh
curl -s \
  -H "Authorization: Bearer eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ1cm46dXVpZDoxNTM2ZTE4OS1kZjk0LTRlM2ItOGZmNS02NTQ0NDM0ZGZhZDciLCJpc3MiOiJkaWQ6d2ViOnBhcnR5LWEtY29ubmVjdG9yJTNBMzAwMCIsImp0aSI6IjQyMDcwOWE4LTYxMzMtNDhiZi1iM2RmLTMyZGI1NDc3M2RiNSIsImlhdCI6MTc4ODE3NDkzNCwiZXhwIjoxNzg4MjAzNzM0fQ.DV9ceG85wYc1KJByPOGJVI_5xhybS2bevrj6lCBoCV0Pe3CgHGY-XU17PAWwf_vCrlmYJrZ-x8zMvxB0B0S0ZA" \
  -H "Content-Type: application/json" \
  http://localhost:13000/pull/test-path?test-query=true \
  -d '{"sample-payload": 1}'
```

Upon success, the echo service behind the demo dataset should echo the request:

```json
{
  "method": "POST",
  "path": "/api/test-path",
  "query": {
    "test-query": "true"
  },
  "headers": {
    "host": "localhost:13000",
    "content-length": "21",
    "user-agent": "curl/8.7.1",
    "accept": "*/*",
    "content-type": "application/json"
  },
  "body": "eyJzYW1wbGUtcGF5bG9hZCI6IDF9"
}
```
