# Proprietary authentication protocol

The sequence diagram below illustrates our proprietary authentication protocol — a protocol based on OpenID4VP that
shares similarities with the [Dataspace Claims Protocol](https://eclipse-dataspace-dcp.github.io/decentralized-claims-protocol/v1.0.1/).

```mermaid
sequenceDiagram
    participant W as Wallet A
    participant A as Connector A
    participant B as Connector B
    participant V as Verifier B

    rect rgba(255, 0, 43, 0.15)
    note over A,B: Authentication Phase
    activate A
    A->>B: POST /auth/verify_me
    activate B
    B->>V: POST /verification-session/create
    activate V
    V-->>B: OpenID4VP request URL + ...

    B-->>A: OpenID4VP request URL + session_id
    deactivate B

    A->>W: POST /credentials/present
    activate W
    deactivate A
    W->>V: POST /verification-session/{session_id}/response
    deactivate W

    activate A
    A->>B: GET /auth/status/{session_id}
    activate B
    B->>V: POST /verification-session/{session_id}/info
    V->>B: Credential Data
    deactivate V
    B->>B: encode_access_token()
    B->>A: <<access_token>> with derived Claims
    deactivate B
    deactivate A
    end

    rect rgba(0, 255, 34, 0.4)
    note over A,B: Negotiation Phase
    activate A
    Note over A,B: Authroization: Bearer <<access_token>>
    A->>B: POST /api/2025/1/negotiations/request
    activate B
    B->>A:
    deactivate B
    deactivate A
    end
```
