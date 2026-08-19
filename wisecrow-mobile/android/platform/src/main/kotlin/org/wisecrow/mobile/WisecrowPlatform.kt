package org.wisecrow.mobile

import android.app.Activity
import android.util.Base64
import java.io.File
import java.io.IOException
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

class WisecrowPlatform internal constructor(
    private val credentials: SecureCredentials? = null,
    private val picker: DocumentPicker? = null,
    private val appPaths: AppPaths? = null,
    private val certificates: CertificateStore? = null,
    private val httpClient: SecureHttpClient? = null,
) {
    constructor(activity: Activity) : this(activity, PlatformResources.create(activity.filesDir))

    private constructor(activity: Activity, resources: PlatformResources?) : this(
        SecureCredentials(activity),
        DocumentPicker(activity),
        resources?.paths,
        resources?.certificates,
        resources?.httpClient,
    )

    fun appDataPath(): String = safe {
        val paths = appPaths ?: return@safe error(PLATFORM_UNAVAILABLE)
        JSONObject()
            .put(STATUS, OK)
            .put(VALUE, paths.root.path)
            .put("database_path", paths.database.path)
            .put("media_path", paths.media.path)
            .put("certificates_path", paths.certificates.path)
            .toString()
    }

    fun credentialLoad(profileId: String): String = safe {
        val store = credentials ?: return@safe error(PLATFORM_UNAVAILABLE)
        when (val result = store.load(profileId)) {
            is CredentialResult.Success -> ok(result.value)
            is CredentialResult.Failure -> error(result.error.name)
        }
    }

    fun credentialSave(profileId: String, token: String): String = safe {
        val store = credentials ?: return@safe error(PLATFORM_UNAVAILABLE)
        credentialUnit(store.save(profileId, token))
    }

    fun credentialDelete(profileId: String): String = safe {
        val store = credentials ?: return@safe error(PLATFORM_UNAVAILABLE)
        credentialUnit(store.delete(profileId))
    }

    fun caImport(profileId: String, certificateBase64: String): String = safe {
        val store = certificates ?: return@safe error(PLATFORM_UNAVAILABLE)
        val client = httpClient ?: return@safe error(PLATFORM_UNAVAILABLE)
        val certificate = decodeCertificate(certificateBase64)
            ?: return@safe error(CertificateError.INVALID_CERTIFICATE.name)
        when (val result = store.save(profileId, certificate)) {
            is CertificateResult.Failure -> error(result.error.name)
            is CertificateResult.Success -> {
                client.invalidateProfile(profileId)
                ok(result.value.fingerprint)
            }
        }
    }

    fun caDelete(profileId: String): String = safe {
        val store = certificates ?: return@safe error(PLATFORM_UNAVAILABLE)
        val client = httpClient ?: return@safe error(PLATFORM_UNAVAILABLE)
        when (val result = store.delete(profileId)) {
            is CertificateResult.Failure -> error(result.error.name)
            is CertificateResult.Success -> {
                client.invalidateProfile(profileId)
                ok()
            }
        }
    }

    fun caFingerprint(profileId: String): String = safe {
        val store = certificates ?: return@safe error(PLATFORM_UNAVAILABLE)
        when (val result = store.load(profileId)) {
            is CertificateResult.Failure -> error(result.error.name)
            is CertificateResult.Success -> ok(result.value?.fingerprint)
        }
    }

    fun caLoad(profileId: String): String = safe {
        val store = certificates ?: return@safe error(PLATFORM_UNAVAILABLE)
        when (val result = store.load(profileId)) {
            is CertificateResult.Failure -> error(result.error.name)
            is CertificateResult.Success -> ok(
                result.value?.let { Base64.encodeToString(it.encoded, Base64.NO_WRAP) },
            )
        }
    }

    fun pickerStart(kind: String, maximumBytes: Long): String = safe {
        val documentPicker = picker ?: return@safe error(PLATFORM_UNAVAILABLE)
        when (val result = documentPicker.start(kind, maximumBytes)) {
            is PickerStartResult.Started -> JSONObject()
                .put(STATUS, PENDING)
                .put(REQUEST_ID, result.requestId)
                .toString()
            is PickerStartResult.Failure -> error(result.error.name)
        }
    }

    fun pickerPoll(requestId: String): String = safe {
        val documentPicker = picker ?: return@safe error(PLATFORM_UNAVAILABLE)
        pickerResult(documentPicker.poll(requestId))
    }

    fun pickerCancel(requestId: String): String = safe {
        val documentPicker = picker ?: return@safe error(PLATFORM_UNAVAILABLE)
        pickerResult(documentPicker.cancel(requestId))
    }

    fun httpStart(requestJson: String): String = safe {
        val client = httpClient ?: return@safe error(PLATFORM_UNAVAILABLE)
        val request = parseHttpRequest(requestJson) ?: return@safe error(HttpError.INVALID_INPUT.name)
        when (val result = client.start(request)) {
            is HttpStartResult.Started -> JSONObject()
                .put(STATUS, PENDING)
                .put(REQUEST_ID, result.requestId)
                .toString()
            is HttpStartResult.Failure -> error(result.error.name)
        }
    }

    fun httpPoll(requestId: String): String = safe {
        val client = httpClient ?: return@safe error(PLATFORM_UNAVAILABLE)
        httpResult(client.poll(requestId))
    }

    fun httpCancel(requestId: String): String = safe {
        val client = httpClient ?: return@safe error(PLATFORM_UNAVAILABLE)
        httpResult(client.cancel(requestId))
    }

    fun syncSchedule(_profileId: String): String = unsupported()

    fun syncCancel(_profileId: String): String = unsupported()

    fun connectivityState(): String = unsupported()

    private fun credentialUnit(result: CredentialResult<Unit>): String = when (result) {
        is CredentialResult.Success -> ok()
        is CredentialResult.Failure -> error(result.error.name)
    }

    private fun pickerResult(result: PickerPollResult): String = when (result) {
        PickerPollResult.Pending -> JSONObject().put(STATUS, PENDING).toString()
        PickerPollResult.Cancelled -> JSONObject().put(STATUS, CANCELLED).toString()
        is PickerPollResult.Failure -> error(result.error.name)
        is PickerPollResult.Ready -> ready(result.file)
    }

    private fun httpResult(result: HttpPollResult): String = when (result) {
        HttpPollResult.Pending -> JSONObject().put(STATUS, PENDING).toString()
        HttpPollResult.Cancelled -> JSONObject().put(STATUS, CANCELLED).toString()
        is HttpPollResult.Failure -> error(result.error.name)
        is HttpPollResult.Ready -> httpReady(result.response)
    }

    private fun httpReady(response: NativeHttpResponse): String {
        val headers = JSONArray()
        response.headers.forEach { (name, value) ->
            headers.put(JSONArray().put(name).put(value))
        }
        return JSONObject()
            .put(STATUS, READY)
            .put("http_status", response.status)
            .put("headers", headers)
            .put("body_base64", Base64.encodeToString(response.body, Base64.NO_WRAP))
            .toString()
    }

    private fun parseHttpRequest(requestJson: String): HttpRequestSpec? {
        if (requestJson.length > MAX_HTTP_JSON_CHARS) return null
        return try {
            val request = JSONObject(requestJson)
            val body = decodeBody(request.getString("body_base64")) ?: return null
            val headers = parseHeaders(request.getJSONArray("headers")) ?: return null
            HttpRequestSpec(
                request.getString("profile_id"),
                request.getString("origin"),
                request.getString("url"),
                request.getString("method"),
                headers,
                body,
                request.getLong("maximum_response_bytes"),
            )
        } catch (_: JSONException) {
            null
        }
    }

    private fun parseHeaders(encoded: JSONArray): List<Pair<String, String>>? {
        if (encoded.length() > MAX_HTTP_HEADERS) return null
        return buildList(encoded.length()) {
            for (index in 0 until encoded.length()) {
                val header = encoded.optJSONArray(index) ?: return null
                if (header.length() != 2) return null
                val name = header.optString(0, null) ?: return null
                val value = header.optString(1, null) ?: return null
                add(name to value)
            }
        }
    }

    private fun decodeBody(encoded: String): ByteArray? {
        if (encoded.length > MAX_ENCODED_REQUEST_BODY_CHARS) return null
        return try {
            Base64.decode(encoded, Base64.NO_WRAP)
        } catch (_: IllegalArgumentException) {
            null
        }
    }

    private fun decodeCertificate(encoded: String): ByteArray? {
        if (encoded.length > MAX_ENCODED_CERTIFICATE_CHARS) return null
        return try {
            Base64.decode(encoded, Base64.NO_WRAP)
        } catch (_: IllegalArgumentException) {
            null
        }
    }

    private fun ready(file: NativePickedFile): String = JSONObject()
        .put(STATUS, READY)
        .put("display_name", file.displayName)
        .put("media_type", file.mediaType)
        .put("bytes_base64", Base64.encodeToString(file.bytes, Base64.NO_WRAP))
        .toString()

    private fun ok(value: String? = null): String {
        val envelope = JSONObject().put(STATUS, OK)
        if (value != null) envelope.put(VALUE, value)
        return envelope.toString()
    }

    private fun error(code: String): String = JSONObject()
        .put(STATUS, ERROR)
        .put(CODE, code)
        .toString()

    private fun unsupported(): String = NOT_IMPLEMENTED

    private inline fun safe(operation: () -> String): String = try {
        operation()
    } catch (_: Exception) {
        error(PLATFORM_FAILURE)
    }

    internal data class AppPaths(
        val root: File,
        val database: File,
        val media: File,
        val certificates: File,
    ) {
        companion object {
            fun create(filesRoot: File): AppPaths? = try {
                val root = filesRoot.canonicalFile
                val database = privateDirectory(root, "database") ?: return null
                val media = privateDirectory(root, "media") ?: return null
                val certificates = privateDirectory(root, "certificates") ?: return null
                AppPaths(root, database, media, certificates)
            } catch (_: IOException) {
                null
            } catch (_: SecurityException) {
                null
            }

            private fun privateDirectory(root: File, name: String): File? {
                val directory = File(root, name).canonicalFile
                if (!directory.toPath().startsWith(root.toPath())) return null
                if (!directory.exists() && !directory.mkdir()) return null
                return directory.takeIf { it.isDirectory }
            }
        }
    }

    private data class PlatformResources(
        val paths: AppPaths,
        val certificates: CertificateStore,
        val httpClient: SecureHttpClient,
    ) {
        companion object {
            fun create(filesRoot: File): PlatformResources? {
                val paths = AppPaths.create(filesRoot) ?: return null
                val certificates = CertificateStore(paths.certificates)
                return PlatformResources(
                    paths,
                    certificates,
                    SecureHttpClient(certificates),
                )
            }
        }
    }

    companion object {
        private const val STATUS = "status"
        private const val CODE = "code"
        private const val VALUE = "value"
        private const val REQUEST_ID = "request_id"
        private const val OK = "OK"
        private const val ERROR = "ERROR"
        private const val PENDING = "PENDING"
        private const val READY = "READY"
        private const val CANCELLED = "CANCELLED"
        private const val PLATFORM_UNAVAILABLE = "PLATFORM_UNAVAILABLE"
        private const val PLATFORM_FAILURE = "PLATFORM_FAILURE"
        private const val MAX_HTTP_HEADERS = 64
        private const val MAX_HTTP_JSON_CHARS = 24_000_000
        private const val MAX_ENCODED_REQUEST_BODY_CHARS = 22_369_624
        private const val MAX_ENCODED_CERTIFICATE_CHARS = 87_384
        const val NOT_IMPLEMENTED = "{\"status\":\"ERROR\",\"code\":\"NOT_IMPLEMENTED\"}"
    }
}
