package org.wisecrow.mobile

import java.io.ByteArrayOutputStream
import java.io.IOException
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLException
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509TrustManager
import okhttp3.Call
import okhttp3.Callback
import okhttp3.CookieJar
import okhttp3.EventListener
import okhttp3.Headers
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response

internal enum class HttpError {
    INVALID_INPUT,
    REQUEST_NOT_FOUND,
    CERTIFICATE_UNAVAILABLE,
    TLS_FAILED,
    NETWORK_FAILED,
    RESPONSE_TOO_LARGE,
    RESPONSE_INVALID,
}

internal data class HttpRequestSpec(
    val profileId: String,
    val origin: String,
    val url: String,
    val method: String,
    val headers: List<Pair<String, String>>,
    val body: ByteArray,
    val maximumResponseBytes: Long,
)

internal data class NativeHttpResponse(
    val status: Int,
    val headers: List<Pair<String, String>>,
    val body: ByteArray,
)

internal sealed interface HttpStartResult {
    data class Started(val requestId: String) : HttpStartResult
    data class Failure(val error: HttpError) : HttpStartResult
}

internal sealed interface HttpRequestState

internal sealed interface HttpPollResult : HttpRequestState {
    data object Pending : HttpPollResult
    data object Cancelled : HttpPollResult
    data class Ready(val response: NativeHttpResponse) : HttpPollResult
    data class Failure(val error: HttpError) : HttpPollResult
}

internal interface HttpClientBoundary {
    fun start(request: HttpRequestSpec): HttpStartResult

    fun poll(requestId: String): HttpPollResult

    fun cancel(requestId: String): HttpPollResult
}

internal class SecureHttpClient(
    private val certificates: CertificateRepository,
    private val platformTrustManager: X509TrustManager = defaultTrustManager(),
) : HttpClientBoundary {
    private val clients = ConcurrentHashMap<ClientKey, OkHttpClient>()
    private val requests = ConcurrentHashMap<String, HttpRequestState>()

    override fun start(request: HttpRequestSpec): HttpStartResult {
        val prepared = when (val result = prepare(request)) {
            is PrepareResult.Failure -> return HttpStartResult.Failure(result.error)
            is PrepareResult.Success -> result.request
        }
        val requestId = UUID.randomUUID().toString()
        val call = prepared.client.newCall(prepared.request)
        val pending = PendingRequest(call, request.profileId, prepared.tlsAttempt)
        requests[requestId] = pending
        return try {
            call.enqueue(callback(requestId, pending, request.maximumResponseBytes))
            HttpStartResult.Started(requestId)
        } catch (_: RuntimeException) {
            requests.remove(requestId, pending)
            call.cancel()
            HttpStartResult.Failure(HttpError.NETWORK_FAILED)
        }
    }

    override fun poll(requestId: String): HttpPollResult {
        if (!validRequestId(requestId)) return HttpPollResult.Failure(HttpError.INVALID_INPUT)
        val state = requests[requestId]
            ?: return HttpPollResult.Failure(HttpError.REQUEST_NOT_FOUND)
        return when (state) {
            is PendingRequest -> HttpPollResult.Pending
            is HttpPollResult -> {
                requests.remove(requestId, state)
                state
            }
        }
    }

    override fun cancel(requestId: String): HttpPollResult {
        if (!validRequestId(requestId)) return HttpPollResult.Failure(HttpError.INVALID_INPUT)
        val state = requests.remove(requestId)
            ?: return HttpPollResult.Failure(HttpError.REQUEST_NOT_FOUND)
        if (state is PendingRequest) state.call.cancel()
        return HttpPollResult.Cancelled
    }

    fun invalidateProfile(profileId: String) {
        clients.entries.removeIf { entry ->
            if (entry.key.profileId != profileId) return@removeIf false
            entry.value.connectionPool.evictAll()
            true
        }
        requests.entries.removeIf { entry ->
            val pending = entry.value as? PendingRequest ?: return@removeIf false
            if (pending.profileId != profileId) return@removeIf false
            pending.call.cancel()
            true
        }
    }

    private fun prepare(spec: HttpRequestSpec): PrepareResult {
        val urls = validateUrls(spec)
            ?: return PrepareResult.Failure(HttpError.INVALID_INPUT)
        if (!validProfile(spec.profileId) || !validBounds(spec)) {
            return PrepareResult.Failure(HttpError.INVALID_INPUT)
        }
        val snapshot = when (val result = certificates.load(spec.profileId)) {
            is CertificateResult.Failure ->
                return PrepareResult.Failure(HttpError.CERTIFICATE_UNAVAILABLE)
            is CertificateResult.Success -> result.value
        }
        val client = try {
            clientFor(spec.profileId, snapshot)
        } catch (_: GeneralSecurityException) {
            return PrepareResult.Failure(HttpError.CERTIFICATE_UNAVAILABLE)
        }
        val tlsAttempt = TlsAttempt()
        val request = buildRequest(spec, urls.request, tlsAttempt)
            ?: return PrepareResult.Failure(HttpError.INVALID_INPUT)
        return PrepareResult.Success(PreparedRequest(client, request, tlsAttempt))
    }

    private fun validateUrls(spec: HttpRequestSpec): ValidatedUrls? {
        val origin = spec.origin.toHttpUrlOrNull() ?: return null
        val request = spec.url.toHttpUrlOrNull() ?: return null
        if (!validHttpsUrl(origin) || !validHttpsUrl(request)) return null
        if (!origin.encodedPath.endsWith('/') || origin.query != null || origin.fragment != null) {
            return null
        }
        val sameOrigin = origin.scheme == request.scheme &&
            origin.host == request.host && origin.port == request.port
        val insidePrefix = request.encodedPath.startsWith(origin.encodedPath)
        return if (sameOrigin && insidePrefix) ValidatedUrls(request) else null
    }

    private fun buildRequest(
        spec: HttpRequestSpec,
        url: HttpUrl,
        tlsAttempt: TlsAttempt,
    ): Request? = try {
        val builder = Request.Builder().url(url).tag(TlsAttempt::class.java, tlsAttempt)
        spec.headers.forEach { (name, value) ->
            if (forbiddenRequestHeader(name)) return null
            builder.addHeader(name, value)
        }
        when (spec.method) {
            "GET" -> {
                if (spec.body.isNotEmpty()) return null
                builder.get()
            }
            "POST" -> builder.post(spec.body.toRequestBody())
            else -> return null
        }
        builder.build()
    } catch (_: IllegalArgumentException) {
        null
    }

    private fun clientFor(
        profileId: String,
        snapshot: CertificateSnapshot?,
    ): OkHttpClient {
        val key = ClientKey(profileId, snapshot?.fingerprint)
        return clients.computeIfAbsent(key) {
            val trustManager = snapshot?.let { certificate ->
                CompositeTrustManager(
                    platformTrustManager,
                    trustManagerFor(certificate.certificate),
                )
            } ?: platformTrustManager
            secureClient(trustManager)
        }
    }

    private fun secureClient(trustManager: X509TrustManager): OkHttpClient {
        val context = SSLContext.getInstance("TLS")
        context.init(null, arrayOf(trustManager), null)
        return OkHttpClient.Builder()
            .sslSocketFactory(context.socketFactory, trustManager)
            .connectTimeout(TIMEOUT_SECONDS, TimeUnit.SECONDS)
            .readTimeout(TIMEOUT_SECONDS, TimeUnit.SECONDS)
            .writeTimeout(TIMEOUT_SECONDS, TimeUnit.SECONDS)
            .callTimeout(CALL_TIMEOUT_SECONDS, TimeUnit.SECONDS)
            .followRedirects(false)
            .followSslRedirects(false)
            .retryOnConnectionFailure(false)
            .cookieJar(CookieJar.NO_COOKIES)
            .eventListenerFactory {
                object : EventListener() {
                    override fun secureConnectStart(call: Call) {
                        call.request().tag(TlsAttempt::class.java)?.started?.set(true)
                    }
                }
            }
            .build()
    }

    private fun callback(
        requestId: String,
        pending: PendingRequest,
        maximumBytes: Long,
    ): Callback = object : Callback {
        override fun onFailure(call: Call, e: IOException) {
            val result = if (call.isCanceled()) {
                HttpPollResult.Cancelled
            } else if (pending.tlsAttempt.started.get() || isTlsFailure(e)) {
                HttpPollResult.Failure(HttpError.TLS_FAILED)
            } else {
                HttpPollResult.Failure(HttpError.NETWORK_FAILED)
            }
            requests.replace(requestId, pending, result)
        }

        override fun onResponse(call: Call, response: Response) {
            val result = response.use { boundedResponse(it, maximumBytes) }
            requests.replace(requestId, pending, result)
        }
    }

    private fun boundedResponse(response: Response, maximumBytes: Long): HttpPollResult {
        val headers = boundedHeaders(response.headers)
            ?: return HttpPollResult.Failure(HttpError.RESPONSE_INVALID)
        val contentLength = response.body.contentLength()
        if (contentLength > maximumBytes) {
            return HttpPollResult.Failure(HttpError.RESPONSE_TOO_LARGE)
        }
        val body = try {
            readBounded(response, maximumBytes)
        } catch (_: IOException) {
            return HttpPollResult.Failure(HttpError.NETWORK_FAILED)
        }
        return when (body) {
            BodyRead.TooLarge -> HttpPollResult.Failure(HttpError.RESPONSE_TOO_LARGE)
            is BodyRead.Success -> HttpPollResult.Ready(
                NativeHttpResponse(response.code, headers, body.bytes),
            )
        }
    }

    private fun readBounded(response: Response, maximumBytes: Long): BodyRead {
        val capacity = minOf(maximumBytes, BUFFER_BYTES.toLong()).toInt()
        val output = ByteArrayOutputStream(capacity)
        val buffer = ByteArray(BUFFER_BYTES)
        response.body.byteStream().use { stream ->
            var total = 0L
            while (true) {
                val count = stream.read(buffer)
                if (count < 0) break
                if (count == 0) {
                    val singleByte = stream.read()
                    if (singleByte < 0) break
                    total += 1L
                    if (total > maximumBytes) return BodyRead.TooLarge
                    output.write(singleByte)
                    continue
                }
                total += count.toLong()
                if (total > maximumBytes) return BodyRead.TooLarge
                output.write(buffer, 0, count)
            }
        }
        return BodyRead.Success(output.toByteArray())
    }

    private fun boundedHeaders(headers: Headers): List<Pair<String, String>>? {
        if (headers.size > MAX_RESPONSE_HEADERS || headers.byteCount() > MAX_HEADER_BYTES) {
            return null
        }
        return headers.map { (name, value) -> name to value }
    }

    private class PendingRequest(
        val call: Call,
        val profileId: String,
        val tlsAttempt: TlsAttempt,
    ) : HttpRequestState

    private class TlsAttempt {
        val started = AtomicBoolean(false)
    }

    private data class ClientKey(val profileId: String, val fingerprint: String?)

    private data class ValidatedUrls(val request: HttpUrl)

    private data class PreparedRequest(
        val client: OkHttpClient,
        val request: Request,
        val tlsAttempt: TlsAttempt,
    )

    private sealed interface PrepareResult {
        data class Success(val request: PreparedRequest) : PrepareResult
        data class Failure(val error: HttpError) : PrepareResult
    }

    private sealed interface BodyRead {
        data object TooLarge : BodyRead
        data class Success(val bytes: ByteArray) : BodyRead
    }

    companion object {
        private const val TIMEOUT_SECONDS = 30L
        private const val CALL_TIMEOUT_SECONDS = 60L
        private const val MAX_RESPONSE_BYTES = 67_108_864L
        private const val MAX_REQUEST_BYTES = 16_777_216
        private const val MAX_REQUEST_HEADERS = 64
        private const val MAX_RESPONSE_HEADERS = 64
        private const val MAX_HEADER_BYTES = 32_768L
        private const val BUFFER_BYTES = 8_192

        private fun validProfile(profileId: String): Boolean = try {
            UUID.fromString(profileId).toString() == profileId
        } catch (_: IllegalArgumentException) {
            false
        }

        private fun validRequestId(requestId: String): Boolean = validProfile(requestId)

        private fun validBounds(spec: HttpRequestSpec): Boolean =
            spec.maximumResponseBytes in 1..MAX_RESPONSE_BYTES &&
                spec.body.size <= MAX_REQUEST_BYTES &&
                spec.headers.size <= MAX_REQUEST_HEADERS &&
                spec.headers.all { (name, value) ->
                    name.length <= MAX_HEADER_FIELD_CHARS &&
                        value.length <= MAX_HEADER_FIELD_CHARS
                } && spec.headers.sumOf { (name, value) ->
                    name.length.toLong() + value.length.toLong()
                } <= MAX_HEADER_BYTES

        private fun validHttpsUrl(url: HttpUrl): Boolean =
            url.scheme == "https" && url.encodedUsername.isEmpty() &&
                url.encodedPassword.isEmpty() && url.fragment == null

        private fun forbiddenRequestHeader(name: String): Boolean = when (name.lowercase()) {
            "connection", "content-length", "host", "transfer-encoding" -> true
            else -> false
        }

        private fun trustManagerFor(certificate: X509Certificate): X509TrustManager {
            val keyStore = KeyStore.getInstance(KeyStore.getDefaultType()).apply { load(null) }
            keyStore.setCertificateEntry("imported", certificate)
            return trustManager(keyStore)
        }

        private fun defaultTrustManager(): X509TrustManager {
            val keyStore: KeyStore? = null
            return trustManager(keyStore)
        }

        private fun trustManager(keyStore: KeyStore?): X509TrustManager {
            val factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm())
            factory.init(keyStore)
            return factory.trustManagers.filterIsInstance<X509TrustManager>().firstOrNull()
                ?: throw GeneralSecurityException("X.509 trust manager is unavailable")
        }

        private fun isTlsFailure(error: Throwable): Boolean {
            var cause: Throwable? = error
            repeat(MAX_CAUSE_DEPTH) {
                if (cause is SSLException || cause is CertificateException) return true
                cause = cause?.cause
            }
            return false
        }

        private const val MAX_CAUSE_DEPTH = 8
        private const val MAX_HEADER_FIELD_CHARS = 8_192
    }
}

private class CompositeTrustManager(
    private val platform: X509TrustManager,
    private val imported: X509TrustManager,
) : X509TrustManager {
    override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) {
        checkBoth(chain, authType, false)
    }

    override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
        checkBoth(chain, authType, true)
    }

    override fun getAcceptedIssuers(): Array<X509Certificate> =
        platform.acceptedIssuers + imported.acceptedIssuers

    private fun checkBoth(
        chain: Array<X509Certificate>,
        authType: String,
        server: Boolean,
    ) {
        try {
            check(platform, chain, authType, server)
            return
        } catch (platformFailure: CertificateException) {
            try {
                check(imported, chain, authType, server)
            } catch (importedFailure: CertificateException) {
                importedFailure.addSuppressed(platformFailure)
                throw importedFailure
            }
        }
    }

    private fun check(
        trustManager: X509TrustManager,
        chain: Array<X509Certificate>,
        authType: String,
        server: Boolean,
    ) {
        if (server) trustManager.checkServerTrusted(chain, authType)
        else trustManager.checkClientTrusted(chain, authType)
    }
}
