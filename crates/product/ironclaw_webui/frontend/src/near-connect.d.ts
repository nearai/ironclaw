declare module "@hot-labs/near-connect" {
  export type NearSignMessageRequest = {
    message: string;
    recipient: string;
    nonce: Uint8Array;
  };

  export type NearSignedMessage = {
    accountId: string;
    publicKey: string;
    signature: string;
  };

  export type NearWallet = {
    signMessage(request: NearSignMessageRequest): Promise<NearSignedMessage>;
  };

  export class NearConnector {
    constructor(options: {
      network: "mainnet" | "testnet";
      features: { signMessage: boolean };
    });

    connect(): Promise<void>;
    wallet(): Promise<NearWallet>;
  }
}
