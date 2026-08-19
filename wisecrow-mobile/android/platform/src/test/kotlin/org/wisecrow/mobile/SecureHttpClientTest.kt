package org.wisecrow.mobile

import java.io.File
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import javax.net.ssl.X509TrustManager
import mockwebserver3.MockResponse
import mockwebserver3.MockWebServer
import okhttp3.HttpUrl
import okhttp3.tls.HandshakeCertificates
import okhttp3.tls.HeldCertificate
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class SecureHttpClientTest {
    @get:Rule
    val temporaryFolder = TemporaryFolder()

    @Test
    fun platformTrustedChainSucceeds() {
        val authority = certificateAuthority("platform-root")
        tlsServer(authority).use { server ->
            server.enqueue(response("platform"))
            val client = client(platformTrustManager(authority.certificate))

            val result = execute(client, request(PROFILE_ONE, server))

            assertEquals("platform", (result as HttpPollResult.Ready).response.body.decodeToString())
        }
    }

    @Test
    fun importedRootIsScopedToOneProfile() {
        val importedAuthority = certificateAuthority("imported-root")
        val unrelatedAuthority = certificateAuthority("platform-root")
        tlsServer(importedAuthority).use { server ->
            val store = certificateStore()
            val saved = store.save(
                PROFILE_ONE,
                importedAuthority.certificatePem().toByteArray(),
            ) as CertificateResult.Success
            assertArrayEquals(importedAuthority.certificate.encoded, saved.value.encoded)
            assertEquals(64, saved.value.fingerprint.length)
            server.enqueue(response("trusted"))
            server.enqueue(response("untrusted"))
            val client = SecureHttpClient(store, platformTrustManager(unrelatedAuthority.certificate))

            val trusted = execute(client, request(PROFILE_ONE, server))
            val untrusted = execute(client, request(PROFILE_TWO, server))

            assertTrue(trusted is HttpPollResult.Ready)
            assertEquals(HttpPollResult.Failure(HttpError.TLS_FAILED), untrusted)
            assertEquals(CertificateResult.Success(Unit), store.delete(PROFILE_ONE))
            assertEquals(CertificateResult.Success(null), store.load(PROFILE_ONE))
        }
    }

    @Test
    fun wrongHostnameAndExpiredLeafFail() {
        val authority = certificateAuthority("imported-root")
        val store = certificateStore()
        assertTrue(store.save(PROFILE_ONE, authority.certificate.encoded) is CertificateResult.Success)
        val client = SecureHttpClient(store, platformTrustManager(certificateAuthority("other").certificate))
        tlsServer(authority).use { server ->
            val wrongHostUrl = server.url("/").newBuilder().host("127.0.0.1").build()
            val wrongHost = execute(client, request(PROFILE_ONE, server, origin = wrongHostUrl))
            assertEquals(HttpPollResult.Failure(HttpError.TLS_FAILED), wrongHost)
        }
        expiredTlsServer(authority).use { server ->
            val expired = execute(client, request(PROFILE_ONE, server))
            assertEquals(HttpPollResult.Failure(HttpError.TLS_FAILED), expired)
        }
    }

    @Test
    fun malformedRootAndHttpUrlFailBeforeNetworkUse() {
        val store = certificateStore()
        assertEquals(
            CertificateResult.Failure(CertificateError.INVALID_CERTIFICATE),
            store.save(PROFILE_ONE, "not-a-certificate".toByteArray()),
        )
        val leaf = validLeaf(certificateAuthority("issuer"))
        assertEquals(
            CertificateResult.Failure(CertificateError.NOT_CERTIFICATE_AUTHORITY),
            store.save(PROFILE_ONE, leaf.certificate.encoded),
        )
        val client = SecureHttpClient(store, platformTrustManager(certificateAuthority("root").certificate))

        val result = client.start(
            HttpRequestSpec(PROFILE_ONE, "http://example.test/", "http://example.test/path", "GET", emptyList(), ByteArray(0), 1_024),
        )

        assertEquals(HttpStartResult.Failure(HttpError.INVALID_INPUT), result)
    }

    @Test
    fun responseAboveRequestedLimitIsAborted() {
        val authority = certificateAuthority("platform-root")
        tlsServer(authority).use { server ->
            server.enqueue(MockResponse.Builder().chunkedBody("too-large", 2).build())
            val client = client(platformTrustManager(authority.certificate))

            val result = execute(client, request(PROFILE_ONE, server, maximumBytes = 3))

            assertEquals(HttpPollResult.Failure(HttpError.RESPONSE_TOO_LARGE), result)
        }
    }

    @Test
    fun cancellationRemovesAndCancelsTheCall() {
        val authority = certificateAuthority("platform-root")
        tlsServer(authority).use { server ->
            server.enqueue(
                MockResponse.Builder()
                    .body("late")
                    .bodyDelay(5, TimeUnit.SECONDS)
                    .build(),
            )
            val client = client(platformTrustManager(authority.certificate))
            val started = client.start(request(PROFILE_ONE, server)) as HttpStartResult.Started

            val cancelled = client.cancel(started.requestId)

            assertEquals(HttpPollResult.Cancelled, cancelled)
            assertEquals(
                HttpPollResult.Failure(HttpError.REQUEST_NOT_FOUND),
                client.poll(started.requestId),
            )
        }
    }

    private fun certificateStore(): CertificateStore =
        CertificateStore(File(temporaryFolder.root, "certificates"))

    private fun client(trustManager: X509TrustManager): SecureHttpClient =
        SecureHttpClient(certificateStore(), trustManager)

    private fun execute(client: SecureHttpClient, request: HttpRequestSpec): HttpPollResult {
        val started = client.start(request)
        if (started !is HttpStartResult.Started) return HttpPollResult.Failure(HttpError.INVALID_INPUT)
        repeat(200) {
            val result = client.poll(started.requestId)
            if (result != HttpPollResult.Pending) return result
            Thread.sleep(10)
        }
        throw AssertionError("HTTP request did not complete")
    }

    private fun request(
        profileId: String,
        server: MockWebServer,
        origin: HttpUrl = server.url("/"),
        maximumBytes: Long = 1_024,
    ): HttpRequestSpec = HttpRequestSpec(
        profileId,
        origin.toString(),
        (origin.resolve("resource") ?: throw AssertionError("test URL is invalid")).toString(),
        "GET",
        emptyList(),
        ByteArray(0),
        maximumBytes,
    )

    private fun response(body: String): MockResponse = MockResponse.Builder().body(body).build()

    private fun tlsServer(authority: HeldCertificate): MockWebServer =
        server(authority, validLeaf(authority))

    private fun expiredTlsServer(authority: HeldCertificate): MockWebServer {
        val now = System.currentTimeMillis()
        val leaf = HeldCertificate.Builder()
            .commonName("localhost")
            .addSubjectAlternativeName("localhost")
            .validityInterval(now - TimeUnit.DAYS.toMillis(2), now - TimeUnit.DAYS.toMillis(1))
            .signedBy(authority)
            .build()
        return server(authority, leaf)
    }

    private fun server(authority: HeldCertificate, leaf: HeldCertificate): MockWebServer {
        val certificates = HandshakeCertificates.Builder()
            .heldCertificate(leaf, authority.certificate)
            .build()
        return MockWebServer().apply {
            useHttps(certificates.sslSocketFactory())
            start()
        }
    }

    private fun validLeaf(authority: HeldCertificate): HeldCertificate = HeldCertificate.Builder()
        .commonName("localhost")
        .addSubjectAlternativeName("localhost")
        .signedBy(authority)
        .build()

    private fun certificateAuthority(name: String): HeldCertificate = HeldCertificate.Builder()
        .certificateAuthority(1)
        .commonName(name)
        .build()

    private fun platformTrustManager(certificate: X509Certificate): X509TrustManager =
        HandshakeCertificates.Builder().addTrustedCertificate(certificate).build().trustManager

    companion object {
        private const val PROFILE_ONE = "00000000-0000-0000-0000-000000000001"
        private const val PROFILE_TWO = "00000000-0000-0000-0000-000000000002"
    }
}
