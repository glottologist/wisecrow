package org.wisecrow.mobile

import java.io.ByteArrayInputStream
import java.io.File
import java.io.IOException
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.util.UUID

internal enum class CertificateError {
    INVALID_INPUT,
    INVALID_CERTIFICATE,
    NOT_CERTIFICATE_AUTHORITY,
    STORAGE_READ_FAILED,
    STORAGE_WRITE_FAILED,
    DELETE_FAILED,
}

internal sealed interface CertificateResult<out T> {
    data class Success<T>(val value: T) : CertificateResult<T>
    data class Failure(val error: CertificateError) : CertificateResult<Nothing>
}

internal data class CertificateSnapshot(
    val fingerprint: String,
    val certificate: X509Certificate,
    val encoded: ByteArray,
)

internal interface CertificateRepository {
    fun save(profileId: String, certificate: ByteArray): CertificateResult<CertificateSnapshot>

    fun load(profileId: String): CertificateResult<CertificateSnapshot?>

    fun delete(profileId: String): CertificateResult<Unit>
}

internal class CertificateStore(root: File) : CertificateRepository {
    private val root = prepareRoot(root)

    @Synchronized
    override fun save(
        profileId: String,
        certificate: ByteArray,
    ): CertificateResult<CertificateSnapshot> {
        val target = try {
            profileFile(profileId)
                ?: return CertificateResult.Failure(CertificateError.INVALID_INPUT)
        } catch (_: IOException) {
            return CertificateResult.Failure(CertificateError.STORAGE_WRITE_FAILED)
        } catch (_: SecurityException) {
            return CertificateResult.Failure(CertificateError.STORAGE_WRITE_FAILED)
        }
        val snapshot = when (val parsed = parseCertificate(certificate)) {
            is CertificateResult.Failure -> return parsed
            is CertificateResult.Success -> parsed.value
        }
        return if (writeAtomically(target, snapshot.encoded)) {
            CertificateResult.Success(snapshot)
        } else {
            CertificateResult.Failure(CertificateError.STORAGE_WRITE_FAILED)
        }
    }

    @Synchronized
    override fun load(profileId: String): CertificateResult<CertificateSnapshot?> {
        val target = try {
            profileFile(profileId)
                ?: return CertificateResult.Failure(CertificateError.INVALID_INPUT)
        } catch (_: IOException) {
            return CertificateResult.Failure(CertificateError.STORAGE_READ_FAILED)
        } catch (_: SecurityException) {
            return CertificateResult.Failure(CertificateError.STORAGE_READ_FAILED)
        }
        if (!target.exists()) return CertificateResult.Success(null)
        val encoded = readBounded(target)
            ?: return CertificateResult.Failure(CertificateError.STORAGE_READ_FAILED)
        return when (val parsed = parseCertificate(encoded)) {
            is CertificateResult.Success -> parsed
            is CertificateResult.Failure ->
                CertificateResult.Failure(CertificateError.STORAGE_READ_FAILED)
        }
    }

    @Synchronized
    override fun delete(profileId: String): CertificateResult<Unit> {
        val target = try {
            profileFile(profileId)
                ?: return CertificateResult.Failure(CertificateError.INVALID_INPUT)
        } catch (_: IOException) {
            return CertificateResult.Failure(CertificateError.DELETE_FAILED)
        } catch (_: SecurityException) {
            return CertificateResult.Failure(CertificateError.DELETE_FAILED)
        }
        if (!target.exists()) return CertificateResult.Success(Unit)
        return if (target.delete()) {
            CertificateResult.Success(Unit)
        } else {
            CertificateResult.Failure(CertificateError.DELETE_FAILED)
        }
    }

    private fun profileFile(profileId: String): File? {
        val profile = canonicalProfile(profileId) ?: return null
        val target = File(root, "$profile.der").canonicalFile
        return target.takeIf { it.toPath().startsWith(root.toPath()) }
    }

    private fun parseCertificate(encoded: ByteArray): CertificateResult<CertificateSnapshot> {
        if (encoded.isEmpty() || encoded.size > MAX_CERTIFICATE_BYTES) {
            return CertificateResult.Failure(CertificateError.INVALID_CERTIFICATE)
        }
        val certificate = parseSingleCertificate(encoded)
            ?: return CertificateResult.Failure(CertificateError.INVALID_CERTIFICATE)
        if (certificate.basicConstraints < 0) {
            return CertificateResult.Failure(CertificateError.NOT_CERTIFICATE_AUTHORITY)
        }
        val der = try {
            certificate.encoded
        } catch (_: CertificateException) {
            return CertificateResult.Failure(CertificateError.INVALID_CERTIFICATE)
        }
        if (der.size > MAX_CERTIFICATE_BYTES) {
            return CertificateResult.Failure(CertificateError.INVALID_CERTIFICATE)
        }
        return CertificateResult.Success(
            CertificateSnapshot(fingerprint(der), certificate, der),
        )
    }

    private fun parseSingleCertificate(encoded: ByteArray): X509Certificate? = try {
        val certificates = CertificateFactory.getInstance("X.509")
            .generateCertificates(ByteArrayInputStream(encoded))
        if (certificates.size == 1) certificates.first() as? X509Certificate else null
    } catch (_: CertificateException) {
        null
    }

    private fun readBounded(target: File): ByteArray? = try {
        if (Files.size(target.toPath()) > MAX_CERTIFICATE_BYTES.toLong()) return null
        Files.readAllBytes(target.toPath()).takeIf { it.size <= MAX_CERTIFICATE_BYTES }
    } catch (_: IOException) {
        null
    } catch (_: SecurityException) {
        null
    }

    private fun writeAtomically(target: File, encoded: ByteArray): Boolean {
        var temporary: File? = null
        return try {
            temporary = File.createTempFile(".certificate-", ".tmp", root)
            temporary.outputStream().use { output ->
                output.write(encoded)
                output.flush()
                output.fd.sync()
            }
            moveIntoPlace(temporary, target)
            true
        } catch (_: IOException) {
            false
        } catch (_: SecurityException) {
            false
        } finally {
            temporary?.takeIf(File::exists)?.delete()
        }
    }

    private fun moveIntoPlace(source: File, target: File) {
        try {
            Files.move(
                source.toPath(),
                target.toPath(),
                StandardCopyOption.ATOMIC_MOVE,
                StandardCopyOption.REPLACE_EXISTING,
            )
        } catch (_: AtomicMoveNotSupportedException) {
            Files.move(source.toPath(), target.toPath(), StandardCopyOption.REPLACE_EXISTING)
        }
    }

    companion object {
        private const val MAX_CERTIFICATE_BYTES = 65_536
        private const val HEX_DIGITS = "0123456789abcdef"

        private fun prepareRoot(directory: File): File {
            val root = directory.canonicalFile
            if (!root.exists() && !root.mkdirs()) {
                throw IOException("certificate directory could not be created")
            }
            if (!root.isDirectory) throw IOException("certificate root is not a directory")
            return root
        }

        private fun canonicalProfile(profileId: String): String? = try {
            val parsed = UUID.fromString(profileId)
            parsed.toString().takeIf { it == profileId }
        } catch (_: IllegalArgumentException) {
            null
        }

        private fun fingerprint(encoded: ByteArray): String {
            val digest = MessageDigest.getInstance("SHA-256").digest(encoded)
            return buildString(digest.size * 2) {
                digest.forEach { byte ->
                    val value = byte.toInt() and 0xff
                    append(HEX_DIGITS[value ushr 4])
                    append(HEX_DIGITS[value and 0x0f])
                }
            }
        }
    }
}
