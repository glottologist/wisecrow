package org.wisecrow.mobile

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.result.ActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.security.cert.CertificateException
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

internal enum class PickerError {
    INVALID_INPUT,
    BUSY,
    REQUEST_NOT_FOUND,
    LAUNCH_FAILED,
    PERMISSION_DENIED,
    METADATA_UNAVAILABLE,
    READ_FAILED,
    FILE_TOO_LARGE,
    WRONG_MIME_TYPE,
    INVALID_FILE,
}

internal sealed interface PickerValidation {
    data object Valid : PickerValidation
    data class Failure(val error: PickerError) : PickerValidation
}

internal sealed interface PickerStartResult {
    data class Started(val requestId: String) : PickerStartResult
    data class Failure(val error: PickerError) : PickerStartResult
}

internal data class NativePickedFile(
    val displayName: String,
    val mediaType: String,
    val bytes: ByteArray,
)

internal sealed interface PickerState

internal sealed interface PickerPollResult : PickerState {
    data object Pending : PickerPollResult
    data object Cancelled : PickerPollResult
    data class Ready(val file: NativePickedFile) : PickerPollResult
    data class Failure(val error: PickerError) : PickerPollResult
}

internal class DocumentPicker private constructor(
    private val context: Context,
    private val executor: Executor,
) {
    private val requests = ConcurrentHashMap<String, PickerState>()
    private val activeRequestId = AtomicReference<String?>(null)
    private lateinit var launchDocument: (Intent) -> Unit

    constructor(activity: Activity) : this(
        activity.applicationContext,
        PICKER_EXECUTOR,
    ) {
        launchDocument = activityLauncher(activity)
    }

    internal constructor(
        context: Context,
        launchDocument: (Intent) -> Unit,
    ) : this(context.applicationContext, Executor { command -> command.run() }) {
        this.launchDocument = launchDocument
    }

    fun start(kind: String, maximumBytes: Long): PickerStartResult {
        val requestError = validateRequest(kind, maximumBytes)
        if (requestError != null) {
            return PickerStartResult.Failure(requestError)
        }
        if (activeRequestId.get() != null) {
            return PickerStartResult.Failure(PickerError.BUSY)
        }
        val requestId = UUID.randomUUID().toString()
        requests[requestId] = PendingRequest(kind, maximumBytes)
        if (!activeRequestId.compareAndSet(null, requestId)) {
            requests.remove(requestId)
            return PickerStartResult.Failure(PickerError.BUSY)
        }
        return launch(requestId, pickerIntent(kind))
    }

    fun poll(requestId: String): PickerPollResult {
        val state = requests[requestId] ?: return PickerPollResult.Failure(
            PickerError.REQUEST_NOT_FOUND,
        )
        return when (state) {
            is PendingRequest -> PickerPollResult.Pending
            is PickerPollResult -> {
                requests.remove(requestId, state)
                state
            }
        }
    }

    fun cancel(requestId: String): PickerPollResult {
        val removed = requests.remove(requestId)
        if (removed == null) {
            return PickerPollResult.Failure(PickerError.REQUEST_NOT_FOUND)
        }
        if (removed is PendingRequest) {
            removed.cancelled.set(true)
        }
        activeRequestId.compareAndSet(requestId, null)
        return PickerPollResult.Cancelled
    }

    internal fun pendingCount(): Int = requests.size

    private fun activityLauncher(activity: Activity): (Intent) -> Unit {
        val owner = activity as? AppCompatActivity ?: return unavailableLauncher()
        return try {
            val launcher = owner.registerForActivityResult(
                ActivityResultContracts.StartActivityForResult(),
            ) { result -> complete(result) }
            val launch: (Intent) -> Unit = { intent -> launcher.launch(intent) }
            launch
        } catch (_: IllegalStateException) {
            unavailableLauncher()
        }
    }

    private fun unavailableLauncher(): (Intent) -> Unit = {
        throw ActivityNotFoundException("document launcher is unavailable")
    }

    private fun launch(requestId: String, intent: Intent): PickerStartResult = try {
        launchDocument(intent)
        PickerStartResult.Started(requestId)
    } catch (_: ActivityNotFoundException) {
        failLaunch(requestId)
    } catch (_: SecurityException) {
        failLaunch(requestId)
    } catch (_: IllegalStateException) {
        failLaunch(requestId)
    }

    private fun failLaunch(requestId: String): PickerStartResult.Failure {
        requests.remove(requestId)
        activeRequestId.compareAndSet(requestId, null)
        return PickerStartResult.Failure(PickerError.LAUNCH_FAILED)
    }

    private fun complete(result: ActivityResult) {
        val requestId = activeRequestId.getAndSet(null) ?: return
        val request = requests[requestId] as? PendingRequest ?: return
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            requests.replace(requestId, request, PickerPollResult.Cancelled)
            return
        }
        try {
            executor.execute {
                if (request.cancelled.get()) return@execute
                val pickedFile = readPickedFile(uri, request)
                requests.replace(requestId, request, pickedFile)
            }
        } catch (_: RejectedExecutionException) {
            requests.replace(
                requestId,
                request,
                PickerPollResult.Failure(PickerError.READ_FAILED),
            )
        }
    }

    private fun readPickedFile(uri: Uri, request: PendingRequest): PickerPollResult {
        if (request.cancelled.get()) return PickerPollResult.Cancelled
        val resolver = context.contentResolver
        try {
            resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        } catch (_: SecurityException) {
            return PickerPollResult.Failure(PickerError.PERMISSION_DENIED)
        }
        val mediaType = resolver.getType(uri)
            ?: return PickerPollResult.Failure(PickerError.METADATA_UNAVAILABLE)
        val displayName = displayName(uri)
            ?: return PickerPollResult.Failure(PickerError.METADATA_UNAVAILABLE)
        val content = when (val bytes = readBounded(uri, effectiveMaximum(request), request)) {
            ReadResult.Cancelled -> return PickerPollResult.Cancelled
            is ReadResult.Failure -> return PickerPollResult.Failure(bytes.error)
            is ReadResult.Success -> bytes.bytes
        }
        return when (val validation = validate(request.kind, mediaType, content, request.maximumBytes)) {
            PickerValidation.Valid -> PickerPollResult.Ready(
                NativePickedFile(displayName, mediaType, content),
            )
            is PickerValidation.Failure -> PickerPollResult.Failure(validation.error)
        }
    }

    private fun displayName(uri: Uri): String? = try {
        context.contentResolver.query(uri, DISPLAY_NAME_COLUMNS, null, null, null)?.use { cursor ->
            val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (index < 0 || !cursor.moveToFirst()) {
                null
            } else {
                cursor.getString(index)?.takeIf { it.isNotBlank() && it.length <= MAX_NAME_CHARS }
            }
        }
    } catch (_: RuntimeException) {
        null
    }

    private fun readBounded(
        uri: Uri,
        maximumBytes: Long,
        request: PendingRequest,
    ): ReadResult = try {
        val input = context.contentResolver.openInputStream(uri)
            ?: return ReadResult.Failure(PickerError.READ_FAILED)
        input.use { stream ->
            val output = ByteArrayOutputStream(BUFFER_BYTES)
            val buffer = ByteArray(BUFFER_BYTES)
            var total = 0L
            while (true) {
                if (request.cancelled.get()) return ReadResult.Cancelled
                val count = stream.read(buffer)
                if (count < 0) break
                if (count == 0) {
                    val singleByte = stream.read()
                    if (singleByte < 0) break
                    total += 1L
                    if (total > maximumBytes) {
                        return ReadResult.Failure(PickerError.FILE_TOO_LARGE)
                    }
                    output.write(singleByte)
                    continue
                }
                total += count.toLong()
                if (total > maximumBytes) {
                    return ReadResult.Failure(PickerError.FILE_TOO_LARGE)
                }
                output.write(buffer, 0, count)
            }
            ReadResult.Success(output.toByteArray())
        }
    } catch (_: IOException) {
        ReadResult.Failure(PickerError.READ_FAILED)
    } catch (_: SecurityException) {
        ReadResult.Failure(PickerError.PERMISSION_DENIED)
    }

    private fun pickerIntent(kind: String): Intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
        addCategory(Intent.CATEGORY_OPENABLE)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        type = if (kind == PDF_KIND) PDF_MIME else "*/*"
        if (kind == CERTIFICATE_KIND) {
            putExtra(Intent.EXTRA_MIME_TYPES, CERTIFICATE_MIME_TYPES)
        }
    }

    private class PendingRequest(
        val kind: String,
        val maximumBytes: Long,
    ) : PickerState {
        val cancelled = AtomicBoolean(false)
    }

    private sealed interface ReadResult {
        data object Cancelled : ReadResult
        data class Success(val bytes: ByteArray) : ReadResult
        data class Failure(val error: PickerError) : ReadResult
    }

    companion object {
        private const val PDF_KIND = "pdf"
        private const val CERTIFICATE_KIND = "certificate"
        private const val PDF_MIME = "application/pdf"
        private const val MAX_CERTIFICATE_BYTES = 65_536L
        private const val MAX_PDF_BYTES = 67_108_864L
        private const val MAX_NAME_CHARS = 255
        private const val BUFFER_BYTES = 8_192
        private val PICKER_EXECUTOR = Executors.newSingleThreadExecutor()
        private val DISPLAY_NAME_COLUMNS = arrayOf(OpenableColumns.DISPLAY_NAME)
        private val PDF_MAGIC = byteArrayOf(0x25, 0x50, 0x44, 0x46, 0x2D)
        private val CERTIFICATE_MIME_TYPES = arrayOf(
            "application/pkix-cert",
            "application/x-x509-ca-cert",
            "application/x-pem-file",
        )

        fun validate(
            kind: String,
            mediaType: String,
            bytes: ByteArray,
            maximumBytes: Long,
        ): PickerValidation {
            val requestError = validateRequest(kind, maximumBytes)
            if (requestError != null) return PickerValidation.Failure(requestError)
            if (bytes.size.toLong() > effectiveMaximum(kind, maximumBytes)) {
                return PickerValidation.Failure(PickerError.FILE_TOO_LARGE)
            }
            return when (kind) {
                PDF_KIND -> validatePdf(mediaType, bytes)
                CERTIFICATE_KIND -> validateCertificate(mediaType, bytes)
                else -> PickerValidation.Failure(PickerError.INVALID_INPUT)
            }
        }

        private fun validateRequest(kind: String, maximumBytes: Long): PickerError? {
            if (kind != PDF_KIND && kind != CERTIFICATE_KIND) return PickerError.INVALID_INPUT
            if (maximumBytes <= 0L || maximumBytes > MAX_PDF_BYTES) return PickerError.INVALID_INPUT
            return null
        }

        private fun effectiveMaximum(request: PendingRequest): Long =
            effectiveMaximum(request.kind, request.maximumBytes)

        private fun effectiveMaximum(kind: String, requested: Long): Long =
            if (kind == CERTIFICATE_KIND) minOf(requested, MAX_CERTIFICATE_BYTES) else requested

        private fun validatePdf(mediaType: String, bytes: ByteArray): PickerValidation {
            if (mediaType != PDF_MIME) {
                return PickerValidation.Failure(PickerError.WRONG_MIME_TYPE)
            }
            val validMagic = bytes.size >= PDF_MAGIC.size &&
                PDF_MAGIC.indices.all { bytes[it] == PDF_MAGIC[it] }
            return if (validMagic) PickerValidation.Valid
            else PickerValidation.Failure(PickerError.INVALID_FILE)
        }

        private fun validateCertificate(mediaType: String, bytes: ByteArray): PickerValidation {
            if (mediaType !in CERTIFICATE_MIME_TYPES) {
                return PickerValidation.Failure(PickerError.WRONG_MIME_TYPE)
            }
            return try {
                val certificates = CertificateFactory.getInstance("X.509")
                    .generateCertificates(ByteArrayInputStream(bytes))
                if (certificates.size == 1 && certificates.first() is X509Certificate) {
                    PickerValidation.Valid
                } else {
                    PickerValidation.Failure(PickerError.INVALID_FILE)
                }
            } catch (_: CertificateException) {
                PickerValidation.Failure(PickerError.INVALID_FILE)
            }
        }
    }
}
