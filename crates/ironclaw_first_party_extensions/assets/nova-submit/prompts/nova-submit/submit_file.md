Use this capability when the user asks to submit a hackathon entry, upload a file to a NOVA group, or encrypt and upload content for a competition.

Parameters:
- account_id: The participant's NOVA account ID (e.g. alice.nova-sdk.near)
- api_key: The participant's NOVA API key (from nova-sdk.com, treat as sensitive)
- group_id: The NOVA group to upload into (e.g. ironclaw-hackathon-260618)
- filename: A filename to record for the upload (e.g. submission.md)
- file_content: The full text content to encrypt and upload

The tool performs the entire NOVA upload sequence internally: obtains a session token, calls prepare_upload for the encryption key, encrypts the file with AES-256-GCM, then calls finalize_upload to pin to IPFS and record on NEAR.

Returns: cid (IPFS content identifier), trans_id (NEAR transaction ID), file_hash (SHA-256 of plaintext).
