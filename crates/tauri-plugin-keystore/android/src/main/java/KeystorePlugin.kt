package dev.flint.keystore

import android.app.Activity
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.util.Base64
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.KeyGenerator
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
class DataArgs {
    var data: String? = null // base64
}

@InvokeArg
class SealedArgs {
    var iv: String? = null // base64
    var ciphertext: String? = null // base64
}

/**
 * Two hardware-backed AES-256-GCM keys:
 *  - `flint_secrets` (seal/open): usable by the app without user action; protects saved secrets such as
 *    the S3 secret key, which are stored only as {iv, ciphertext}.
 *  - `flint_dek_wrap` (wrap/unwrap): usable only after a fingerprint, for every single use; protects a
 *    store's data key (DEK), stored only as {iv, ciphertext}.
 */
@TauriPlugin
class KeystorePlugin(private val activity: Activity) : Plugin(activity) {
    private val alias = "flint_dek_wrap"
    private val secretsAlias = "flint_secrets"

    private fun b64(bytes: ByteArray) = Base64.encodeToString(bytes, Base64.NO_WRAP)
    private fun unb64(s: String) = Base64.decode(s, Base64.NO_WRAP)

    private fun keyStore(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun existingKey(): SecretKey? = keyStore().getKey(alias, null) as? SecretKey

    private fun createKey(): SecretKey {
        fun spec(strongBox: Boolean): KeyGenParameterSpec {
            val b = KeyGenParameterSpec.Builder(
                alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setUserAuthenticationRequired(true)
                .setInvalidatedByBiometricEnrollment(true)
            if (Build.VERSION.SDK_INT >= 30) {
                // timeout 0 = authenticate for every single use, through a CryptoObject
                b.setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
            } else {
                b.setUserAuthenticationValidityDurationSeconds(-1)
            }
            if (strongBox && Build.VERSION.SDK_INT >= 28) b.setIsStrongBoxBacked(true)
            return b.build()
        }

        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        return try {
            gen.init(spec(strongBox = true))
            gen.generateKey()
        } catch (e: StrongBoxUnavailableException) {
            gen.init(spec(strongBox = false))
            gen.generateKey()
        }
    }

    /** The key for [seal]/[open]: hardware-backed (StrongBox when there is one), no user action needed. */
    private fun secretsKey(): SecretKey {
        (keyStore().getKey(secretsAlias, null) as? SecretKey)?.let { return it }
        fun spec(strongBox: Boolean): KeyGenParameterSpec {
            val b = KeyGenParameterSpec.Builder(
                secretsAlias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
            if (strongBox && Build.VERSION.SDK_INT >= 28) b.setIsStrongBoxBacked(true)
            return b.build()
        }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        return try {
            gen.init(spec(strongBox = true))
            gen.generateKey()
        } catch (e: StrongBoxUnavailableException) {
            gen.init(spec(strongBox = false))
            gen.generateKey()
        }
    }

    private fun biometricStatus(): String =
        when (BiometricManager.from(activity).canAuthenticate(BIOMETRIC_STRONG)) {
            BiometricManager.BIOMETRIC_SUCCESS -> "ready"
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> "none_enrolled"
            BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE -> "no_hardware"
            BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE -> "hw_unavailable"
            BiometricManager.BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED -> "security_update_required"
            else -> "unsupported"
        }

    private fun authenticate(invoke: Invoke, cipher: Cipher, title: String, onOk: (Cipher) -> Unit) {
        val host = activity as? FragmentActivity
        if (host == null) {
            invoke.reject("activity is not a FragmentActivity", "no_fragment_activity")
            return
        }
        host.runOnUiThread {
            val callback = object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    try {
                        onOk(result.cryptoObject!!.cipher!!)
                    } catch (e: Exception) {
                        invoke.reject(e.toString(), "crypto_failed")
                    }
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    // 10 = user cancelled, 13 = negative button ("Use password"), 7/9 = lockout
                    invoke.reject(errString.toString(), "auth_error_$errorCode")
                }
                // onAuthenticationFailed (a wrong finger) keeps the prompt open; nothing to do.
            }
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle(title)
                .setSubtitle("Flint")
                .setAllowedAuthenticators(BIOMETRIC_STRONG)
                .setNegativeButtonText("Use password")
                .build()
            BiometricPrompt(host, ContextCompat.getMainExecutor(host), callback)
                .authenticate(info, BiometricPrompt.CryptoObject(cipher))
        }
    }

    @Command
    fun status(invoke: Invoke) {
        val ret = JSObject()
        ret.put("biometric", biometricStatus())
        ret.put("sdk", Build.VERSION.SDK_INT)
        try {
            val key = existingKey()
            ret.put("keyExists", key != null)
            if (key != null) {
                val info = SecretKeyFactory.getInstance(key.algorithm, "AndroidKeyStore")
                    .getKeySpec(key, KeyInfo::class.java) as KeyInfo
                ret.put("insideSecureHardware", info.isInsideSecureHardware)
                if (Build.VERSION.SDK_INT >= 31) {
                    ret.put(
                        "securityLevel", when (info.securityLevel) {
                            KeyProperties.SECURITY_LEVEL_STRONGBOX -> "strongbox"
                            KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT -> "tee"
                            KeyProperties.SECURITY_LEVEL_SOFTWARE -> "software"
                            else -> "unknown"
                        }
                    )
                }
                ret.put("userAuthenticationRequired", info.isUserAuthenticationRequired)
                ret.put("invalidatedByBiometricEnrollment", info.isInvalidatedByBiometricEnrollment)
            }
            invoke.resolve(ret)
        } catch (e: Exception) {
            invoke.reject(e.toString(), "status_failed")
        }
    }

    @Command
    fun seal(invoke: Invoke) {
        val args = invoke.parseArgs(DataArgs::class.java)
        val data = args.data?.let { unb64(it) }
        if (data == null) {
            invoke.reject("data is required", "bad_args")
            return
        }
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, secretsKey()) // Keystore picks a fresh random IV
            val ret = JSObject()
            ret.put("ciphertext", b64(cipher.doFinal(data)))
            ret.put("iv", b64(cipher.iv))
            invoke.resolve(ret)
        } catch (e: Exception) {
            invoke.reject(e.toString(), "seal_failed")
        }
    }

    @Command
    fun open(invoke: Invoke) {
        val args = invoke.parseArgs(SealedArgs::class.java)
        val iv = args.iv?.let { unb64(it) }
        val ct = args.ciphertext?.let { unb64(it) }
        if (iv == null || ct == null) {
            invoke.reject("iv and ciphertext are required", "bad_args")
            return
        }
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, secretsKey(), GCMParameterSpec(128, iv))
            val ret = JSObject()
            ret.put("data", b64(cipher.doFinal(ct)))
            invoke.resolve(ret)
        } catch (e: Exception) {
            invoke.reject(e.toString(), "open_failed")
        }
    }

    @Command
    fun wrap(invoke: Invoke) {
        val args = invoke.parseArgs(DataArgs::class.java)
        val data = args.data?.let { unb64(it) }
        if (data == null) {
            invoke.reject("data is required", "bad_args")
            return
        }
        try {
            val key = existingKey() ?: createKey()
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, key) // Keystore picks a fresh random IV
            authenticate(invoke, cipher, "Turn on fingerprint unlock") { c ->
                val ct = c.doFinal(data)
                val ret = JSObject()
                ret.put("iv", b64(c.iv))
                ret.put("ciphertext", b64(ct))
                invoke.resolve(ret)
            }
        } catch (e: KeyPermanentlyInvalidatedException) {
            invoke.reject("key invalidated by a biometric change", "key_invalidated")
        } catch (e: Exception) {
            invoke.reject(e.toString(), "wrap_failed")
        }
    }

    @Command
    fun unwrap(invoke: Invoke) {
        val args = invoke.parseArgs(SealedArgs::class.java)
        val iv = args.iv?.let { unb64(it) }
        val ct = args.ciphertext?.let { unb64(it) }
        if (iv == null || ct == null) {
            invoke.reject("iv and ciphertext are required", "bad_args")
            return
        }
        try {
            val key = existingKey()
            if (key == null) {
                invoke.reject("no key; wrap first", "no_key")
                return
            }
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, iv))
            authenticate(invoke, cipher, "Unlock your store") { c ->
                val ret = JSObject()
                ret.put("data", b64(c.doFinal(ct)))
                invoke.resolve(ret)
            }
        } catch (e: KeyPermanentlyInvalidatedException) {
            invoke.reject("key invalidated by a biometric change", "key_invalidated")
        } catch (e: Exception) {
            invoke.reject(e.toString(), "unwrap_failed")
        }
    }

    @Command
    fun forget(invoke: Invoke) {
        try {
            keyStore().deleteEntry(alias)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject(e.toString(), "forget_failed")
        }
    }
}
