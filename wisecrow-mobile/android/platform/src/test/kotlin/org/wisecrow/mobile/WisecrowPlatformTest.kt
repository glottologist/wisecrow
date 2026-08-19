package org.wisecrow.mobile

import org.junit.Assert.assertTrue
import org.junit.Test

class WisecrowPlatformTest {
    private val platform = WisecrowPlatform()

    @Test
    fun initializedOperationsRequireAPlatformContext() {
        val responses = listOf(
            platform.appDataPath(),
            platform.credentialLoad("profile"),
            platform.credentialSave("profile", "token"),
            platform.credentialDelete("profile"),
            platform.pickerStart("pdf", 1_024),
            platform.pickerPoll("request"),
            platform.pickerCancel("request"),
            platform.caImport("profile", "certificate"),
            platform.caDelete("profile"),
            platform.caFingerprint("profile"),
            platform.caLoad("profile"),
            platform.httpStart("{}"),
            platform.httpPoll("request"),
            platform.httpCancel("request"),
        )

        assertTrue(responses.all { it.contains("PLATFORM_UNAVAILABLE") })
    }

    @Test
    fun laterPhaseOperationsRemainNotImplemented() {
        val responses = listOf(
            platform.syncSchedule("profile"),
            platform.syncCancel("profile"),
            platform.connectivityState(),
        )

        assertTrue(responses.all { it == WisecrowPlatform.NOT_IMPLEMENTED })
    }
}
