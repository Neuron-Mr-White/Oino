package dev.unipi.app

import android.util.Base64
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * UNI-161 §1/§2: decrypts the `unipi1:` AES-256-GCM envelope ntfy/
 * UnifiedPush carries, for background delivery (the WebView's own
 * WebCrypto can't run while the app process has no page loaded).
 * Format and the pi/host-side producers are documented in
 * `docs/m6/PROTOCOL.md`: `unipi1:` + base64(iv(12) || ciphertext || tag(16)).
 *
 * Mirrors `apps/mobile/src/lib/notifyCrypto.ts` (WebView WebCrypto path)
 * and `packages/notify/ntfy-crypto.ts` (pi-side encrypt) byte-for-byte.
 */
object NotifyCrypto {
  const val ENVELOPE_PREFIX = "unipi1:"

  class NotifyPayload(
    val title: String,
    val body: String,
    val kind: String?,
    val host: String?,
    val pid: Int?,
    val session: String?,
    val dialogId: Int?,
    val options: List<String>?,
  )

  fun isEncryptedEnvelope(message: String?): Boolean = message != null && message.startsWith(ENVELOPE_PREFIX)

  /** Decrypts one envelope with `keyB64` (32 raw bytes, base64 standard).
   *  Throws on a bad prefix, length, key or auth tag — callers must treat
   *  any exception as "drop this notification", never show ciphertext. */
  @Throws(Exception::class)
  fun decrypt(keyB64: String, envelope: String): NotifyPayload {
    require(isEncryptedEnvelope(envelope)) { "missing unipi1: prefix" }
    val combined = Base64.decode(envelope.substring(ENVELOPE_PREFIX.length), Base64.DEFAULT)
    require(combined.size >= 12 + 16) { "envelope too short" }
    val iv = combined.copyOfRange(0, 12)
    // javax.crypto wants ciphertext+tag concatenated, same as the bytes after the IV.
    val ciphertextAndTag = combined.copyOfRange(12, combined.size)
    val keyBytes = Base64.decode(keyB64, Base64.DEFAULT)
    require(keyBytes.size == 32) { "notify key must be 32 bytes" }

    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    val spec = GCMParameterSpec(128, iv)
    cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(keyBytes, "AES"), spec)
    val plaintext = cipher.doFinal(ciphertextAndTag)
    val json = org.json.JSONObject(String(plaintext, Charsets.UTF_8))
    val options = json.optJSONArray("options")?.let { array ->
      (0 until array.length()).map { array.getString(it) }
    }
    return NotifyPayload(
      title = json.optString("title", ""),
      body = json.optString("body", ""),
      kind = if (json.has("kind")) json.optString("kind") else null,
      host = if (json.has("host")) json.optString("host") else null,
      pid = if (json.has("pid")) json.optInt("pid") else null,
      session = if (json.has("session")) json.optString("session") else null,
      dialogId = if (json.has("dialogId")) json.optInt("dialogId") else null,
      options = options,
    )
  }
}
