package org.wisecrow.mobile

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.ProviderException
import java.util.UUID
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

internal enum class CredentialError {
    INVALID_INPUT,
    KEYSTORE_UNAVAILABLE,
    CREDENTIAL_CORRUPT,
    AUTHENTICATION_FAILED,
    STORAGE_READ_FAILED,
    STORAGE_WRITE_FAILED,
    DELETE_FAILED,
}

internal sealed interface CredentialResult<out T> {
    data class Success<T>(val value: T) : CredentialResult<T>
    data class Failure(val error: CredentialError) : CredentialResult<Nothing>
}

internal class SecureCredentials(
    context: Context,
    private val keyAlias: String = DEFAULT_KEY_ALIAS,
    preferencesName: String = DEFAULT_PREFERENCES_NAME,
) {
    private val preferences =
        context.applicationContext.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)

    fun save(profileId: String, token: String): CredentialResult<Unit> {
        val authenticatedProfile = authenticatedProfile(profileId)
            ?: return CredentialResult.Failure(CredentialError.INVALID_INPUT)
        val cleartext = token.toByteArray(StandardCharsets.UTF_8)
        if (cleartext.isEmpty() || cleartext.size > MAX_TOKEN_BYTES) {
            cleartext.fill(0)
            return CredentialResult.Failure(CredentialError.INVALID_INPUT)
        }
        return try {
            val encoded = encrypt(authenticatedProfile, cleartext)
            if (preferences.edit().putString(storageKey(profileId), encoded).commit()) {
                CredentialResult.Success(Unit)
            } else {
                CredentialResult.Failure(CredentialError.STORAGE_WRITE_FAILED)
            }
        } catch (_: GeneralSecurityException) {
            CredentialResult.Failure(CredentialError.KEYSTORE_UNAVAILABLE)
        } catch (_: ProviderException) {
            CredentialResult.Failure(CredentialError.KEYSTORE_UNAVAILABLE)
        } catch (_: IOException) {
            CredentialResult.Failure(CredentialError.KEYSTORE_UNAVAILABLE)
        } finally {
            cleartext.fill(0)
        }
    }

    fun load(profileId: String): CredentialResult<String?> {
        val authenticatedProfile = authenticatedProfile(profileId)
            ?: return CredentialResult.Failure(CredentialError.INVALID_INPUT)
        val encoded = try {
            preferences.getString(storageKey(profileId), null)
        } catch (_: ClassCastException) {
            return CredentialResult.Failure(CredentialError.STORAGE_READ_FAILED)
        } ?: return CredentialResult.Success(null)
        val stored = decodeStoredValue(encoded)
            ?: return CredentialResult.Failure(CredentialError.CREDENTIAL_CORRUPT)
        return decrypt(authenticatedProfile, stored)
    }

    fun delete(profileId: String): CredentialResult<Unit> {
        if (authenticatedProfile(profileId) == null) {
            return CredentialResult.Failure(CredentialError.INVALID_INPUT)
        }
        return if (preferences.edit().remove(storageKey(profileId)).commit()) {
            CredentialResult.Success(Unit)
        } else {
            CredentialResult.Failure(CredentialError.DELETE_FAILED)
        }
    }

    private fun encrypt(authenticatedProfile: ByteArray, cleartext: ByteArray): String {
        val iv = ByteArray(IV_BYTES)
        java.security.SecureRandom().nextBytes(iv)
        val cipher = Cipher.getInstance(CIPHER_TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey(), GCMParameterSpec(TAG_BITS, iv))
        cipher.updateAAD(authenticatedProfile)
        val ciphertext = cipher.doFinal(cleartext)
        val stored = ByteArray(HEADER_BYTES + ciphertext.size)
        stored[0] = FORMAT_VERSION
        iv.copyInto(stored, 1)
        ciphertext.copyInto(stored, HEADER_BYTES)
        return Base64.encodeToString(stored, Base64.NO_WRAP)
    }

    private fun decrypt(
        authenticatedProfile: ByteArray,
        stored: ByteArray,
    ): CredentialResult<String> {
        val iv = stored.copyOfRange(1, HEADER_BYTES)
        val ciphertext = stored.copyOfRange(HEADER_BYTES, stored.size)
        var cleartext: ByteArray? = null
        return try {
            val cipher = Cipher.getInstance(CIPHER_TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(TAG_BITS, iv))
            cipher.updateAAD(authenticatedProfile)
            cleartext = cipher.doFinal(ciphertext)
            decodeToken(cleartext)
        } catch (_: AEADBadTagException) {
            CredentialResult.Failure(CredentialError.AUTHENTICATION_FAILED)
        } catch (_: GeneralSecurityException) {
            CredentialResult.Failure(CredentialError.KEYSTORE_UNAVAILABLE)
        } catch (_: ProviderException) {
            CredentialResult.Failure(CredentialError.KEYSTORE_UNAVAILABLE)
        } catch (_: IOException) {
            CredentialResult.Failure(CredentialError.KEYSTORE_UNAVAILABLE)
        } finally {
            cleartext?.fill(0)
        }
    }

    private fun decodeToken(cleartext: ByteArray): CredentialResult<String> = try {
        val decoder = StandardCharsets.UTF_8.newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
        CredentialResult.Success(decoder.decode(ByteBuffer.wrap(cleartext)).toString())
    } catch (_: java.nio.charset.CharacterCodingException) {
        CredentialResult.Failure(CredentialError.CREDENTIAL_CORRUPT)
    }

    private fun decodeStoredValue(encoded: String): ByteArray? {
        val stored = try {
            Base64.decode(encoded, Base64.NO_WRAP)
        } catch (_: IllegalArgumentException) {
            return null
        }
        return stored.takeIf {
            it.size >= HEADER_BYTES + TAG_BYTES && it[0] == FORMAT_VERSION
        }
    }

    @Synchronized
    private fun secretKey(): SecretKey {
        val keyStore = KeyStore.getInstance(KEYSTORE_PROVIDER).apply { load(null) }
        val existing = keyStore.getKey(keyAlias, null)
        if (existing is SecretKey) {
            return existing
        }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE_PROVIDER)
        val specification = KeyGenParameterSpec.Builder(
            keyAlias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setKeySize(KEY_BITS)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setRandomizedEncryptionRequired(true)
            .build()
        generator.init(specification)
        return generator.generateKey()
    }

    private fun authenticatedProfile(profileId: String): ByteArray? = try {
        val parsed = UUID.fromString(profileId)
        profileId.takeIf { parsed.toString() == it }?.toByteArray(StandardCharsets.UTF_8)
    } catch (_: IllegalArgumentException) {
        null
    }

    companion object {
        private const val DEFAULT_KEY_ALIAS = "org.wisecrow.mobile.credentials.v1"
        private const val DEFAULT_PREFERENCES_NAME = "wisecrow_secure_credentials"
        private const val KEYSTORE_PROVIDER = "AndroidKeyStore"
        private const val CIPHER_TRANSFORMATION = "AES/GCM/NoPadding"
        private const val KEY_BITS = 256
        private const val TAG_BITS = 128
        private const val TAG_BYTES = TAG_BITS / 8
        private const val IV_BYTES = 12
        private const val HEADER_BYTES = 1 + IV_BYTES
        private const val MAX_TOKEN_BYTES = 65_536
        private const val FORMAT_VERSION: Byte = 1

        fun storageKey(profileId: String): String = "credential_$profileId"
    }
}
