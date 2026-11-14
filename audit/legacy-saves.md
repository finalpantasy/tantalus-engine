# Legacy PC and Memoria save audit

Reference: local Memoria source snapshot `c716e1faaeee8c856cc10921234d017f1187b45b`. All findings below are source observations, not real-save runtime verification.

## Original PC base container

- `Assembly-CSharp/Global/Shared/Data/SharedDataBytesStorage.cs:565-668` reads metadata and one selected save block, decrypts it, and expects a decrypted payload of `DataSize + 4` beginning with `SAVE` or `NONE`.
- `SharedDataBytesStorage.cs:1285-1466` defines a 288-byte plaintext metadata structure, 320-byte metadata reservation, 10 slots, 15 saves per slot, 1,024-byte preview reservations, and 18,432-byte save blocks. `BaseSaveBlockOffset` is 153,920. The fixed PC container size derived from those constants is 2,937,152 bytes, including the autosave reservation.
- `Assembly-CSharp/Global/AESCryptography.cs:7-100` defines AES/Rijndael with a 256-bit key, 128-bit blocks, CBC mode, PBKDF2 (`Rfc2898DeriveBytes`) with 1,000 iterations, and a fixed password/salt. This is format compatibility information, not a security feature for protecting user data.
- `Assembly-CSharp/Global/JsonParser.cs:33-83` and the remaining parser map game state into a schema-driven typed tree. `JsonParser` is therefore not evidence of a plain-text JSON disk format.

Base AES decoding and complete schema mapping are not yet implemented in Tantalus. `LegacySaveImporter::inspect` reports bytes, SHA-256, expected fixed-size agreement, and `decoded: false`; it does not infer validity from size alone.

## Memoria extra data

- `SharedDataBytesStorage.cs:649-659` loads an optional sidecar through `JSONNode.LoadFromFile` and adds slot/save/file metadata only to the in-memory tree.
- `SharedDataBytesStorage.cs:671-735` removes `MemoriaExtraData` from the base tree before writing the base block, then writes the sidecar separately.
- `Assembly-CSharp/SimpleJSON/JSONNode.cs:511-518,557-597,614-631` shows recursive binary serialization using little-endian `BinaryWriter`/`BinaryReader` values. Tags are array 1, class 2, string 3, Int32 4, Double 5, Boolean 6, and Float 7; strings use the .NET seven-bit-length-prefixed UTF-8 representation.
- `SimpleJSON/JSONData.cs:55-88`, `JSONArray.cs:132-140`, and `JSONClass.cs:202-211` define value and collection encoding.
- `SharedDataBytesStorage.cs:1460-1466` derives `<base>_Memoria_<slot>_<save>.dat` or `<base>_Memoria_Autosave.dat`. The stored sidecar does not independently prove its base pairing. Tantalus therefore validates the expected filename against explicit slot/save coordinates and otherwise leaves pairing unchecked.

Tantalus rejects duplicate class keys, excessive nesting/nodes, invalid UTF-8, truncation, and unknown tags with byte locations rather than silently replacing required state. Trailing bytes are counted because the reference loader does not demand immediate EOF. The encoder/decoder tests use only project-owned synthetic data.

## Compatibility limits

No owned original or Memoria save has been supplied. Real decryption, field mapping, sidecar/base behavior, and export are `NOT_RUN`. No legacy source file is modified by the inspector. Original save writing in the reference is a separate base/sidecar operation and is not used as a model for native atomicity.
