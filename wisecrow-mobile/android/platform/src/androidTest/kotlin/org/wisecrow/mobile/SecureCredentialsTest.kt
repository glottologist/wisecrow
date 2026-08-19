package org.wisecrow.mobile

import android.content.Context
import android.util.Base64
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.security.KeyStore
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SecureCredentialsTest {
    private lateinit var context: Context
    private lateinit var credentials: SecureCredentials

    @Before
    fun setUp() {
        context = ApplicationProvider.getApplicationContext()
        clearTestStorage()
        credentials = SecureCredentials(context, KEY_ALIAS, PREFERENCES_NAME)
    }

    @After
    fun tearDown() {
        clearTestStorage()
    }

    @Test
    fun credentialsAreEncryptedOverwritableDeletableAndProfileScoped() {
        assertEquals(CredentialResult.Success(Unit), credentials.save(PROFILE_ONE, "first-token"))
        assertEquals(CredentialResult.Success(Unit), credentials.save(PROFILE_TWO, "second-token"))
        assertEquals(CredentialResult.Success("first-token"), credentials.load(PROFILE_ONE))
        assertEquals(CredentialResult.Success("second-token"), credentials.load(PROFILE_TWO))

        val ciphertext = preferences().getString(SecureCredentials.storageKey(PROFILE_ONE), null)
        assertFalse(ciphertext.orEmpty().contains("first-token"))
        assertEquals(CredentialResult.Success(Unit), credentials.save(PROFILE_ONE, "replacement"))
        assertEquals(CredentialResult.Success("replacement"), credentials.load(PROFILE_ONE))

        assertEquals(CredentialResult.Success(Unit), credentials.delete(PROFILE_ONE))
        assertEquals(CredentialResult.Success(null), credentials.load(PROFILE_ONE))
        assertEquals(CredentialResult.Success("second-token"), credentials.load(PROFILE_TWO))
    }

    @Test
    fun malformedCiphertextReturnsATypedReadError() {
        assertTrue(
            preferences()
                .edit()
                .putString(SecureCredentials.storageKey(PROFILE_ONE), "not-base64")
                .commit(),
        )

        assertEquals(
            CredentialResult.Failure(CredentialError.CREDENTIAL_CORRUPT),
            credentials.load(PROFILE_ONE),
        )
    }

    @Test
    fun modifiedAuthenticationTagReturnsAuthenticationFailure() {
        assertEquals(CredentialResult.Success(Unit), credentials.save(PROFILE_ONE, "secret-token"))
        val encoded = preferences().getString(SecureCredentials.storageKey(PROFILE_ONE), null)
            ?: throw AssertionError("encrypted credential was not stored")
        val tampered = Base64.decode(encoded, Base64.NO_WRAP)
        val lastIndex = tampered.lastIndex
        tampered[lastIndex] = (tampered[lastIndex].toInt() xor 1).toByte()
        assertTrue(
            preferences()
                .edit()
                .putString(
                    SecureCredentials.storageKey(PROFILE_ONE),
                    Base64.encodeToString(tampered, Base64.NO_WRAP),
                )
                .commit(),
        )

        assertEquals(
            CredentialResult.Failure(CredentialError.AUTHENTICATION_FAILED),
            credentials.load(PROFILE_ONE),
        )
    }

    @Test
    fun appPathsRemainInsideCanonicalPrivateStorage() {
        val paths = WisecrowPlatform.AppPaths.create(context.filesDir)
            ?: throw AssertionError("private paths were not created")

        assertEquals(context.filesDir.canonicalFile, paths.root)
        assertTrue(paths.database.toPath().startsWith(paths.root.toPath()))
        assertTrue(paths.media.toPath().startsWith(paths.root.toPath()))
        assertTrue(paths.certificates.toPath().startsWith(paths.root.toPath()))
    }

    @Test
    fun pickerRejectsOversizeWrongTypeAndCancellation() {
        val oversizedPdf = DocumentPicker.validate("pdf", "application/pdf", ByteArray(6), 5)
        val oversizedCa = DocumentPicker.validate(
            "certificate",
            "application/pkix-cert",
            ByteArray(65_537),
            100_000,
        )
        val wrongMime = DocumentPicker.validate(
            "pdf",
            "text/plain",
            "%PDF-1.7".toByteArray(),
            1_024,
        )

        assertEquals(PickerValidation.Failure(PickerError.FILE_TOO_LARGE), oversizedPdf)
        assertEquals(PickerValidation.Failure(PickerError.FILE_TOO_LARGE), oversizedCa)
        assertEquals(PickerValidation.Failure(PickerError.WRONG_MIME_TYPE), wrongMime)

        val picker = DocumentPicker(context) { }
        val started = picker.start("pdf", 1_024)
        assertTrue(started is PickerStartResult.Started)
        val requestId = (started as PickerStartResult.Started).requestId
        assertEquals(PickerPollResult.Cancelled, picker.cancel(requestId))
        assertEquals(0, picker.pendingCount())
    }

    private fun preferences() =
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)

    private fun clearTestStorage() {
        assertTrue(preferences().edit().clear().commit())
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (keyStore.containsAlias(KEY_ALIAS)) {
            keyStore.deleteEntry(KEY_ALIAS)
        }
    }

    companion object {
        private const val KEY_ALIAS = "wisecrow-test-credentials"
        private const val PREFERENCES_NAME = "wisecrow-test-credentials"
        private const val PROFILE_ONE = "00000000-0000-0000-0000-000000000001"
        private const val PROFILE_TWO = "00000000-0000-0000-0000-000000000002"
    }
}
