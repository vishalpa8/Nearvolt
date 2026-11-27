package app.nearvolt

import android.annotation.SuppressLint
import android.content.Context
import android.net.wifi.WifiManager
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pGroup
import android.net.wifi.p2p.WifiP2pInfo
import android.net.wifi.p2p.WifiP2pManager
import android.os.Looper
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import java.security.SecureRandom
import kotlin.coroutines.resume

/**
 * Wi-Fi Direct for devices that aren't on the same network. The receiver creates
 * the group itself and shares its name, password and channel in the ticket, so
 * the sender joins directly: no device search and no confirmation pop-up.
 */
@SuppressLint("MissingPermission") // Callers request the nearby-devices permission first.
class WifiDirect(private val context: Context) {
    private val manager = context.getSystemService(WifiP2pManager::class.java)
    private val channel by lazy { manager?.initialize(context, Looper.getMainLooper(), null) }

    /**
     * Creates a group this device owns. Files go straight through it, so its
     * speed is the transfer's speed: a 5 GHz channel is several times faster
     * than 2.4 GHz. If the router is on a usable 5 GHz channel the group shares
     * it, so the radio never hops between channels; otherwise it asks for any
     * 5 GHz channel, then for the router's channel, then lets Android choose.
     */
    suspend fun createGroup(): WifiGroup? {
        val manager = manager ?: return null
        val channel = channel ?: return null
        if (!clear(manager, channel)) return null
        val name = PREFIX + random(4)
        val passphrase = random(12)
        val router = routerFrequency()
        val attempts = buildList<Pair<String, WifiP2pConfig.Builder.() -> Unit>> {
            if (router != null && router >= FIVE_GHZ) add("the router's 5 GHz channel $router MHz" to { setGroupOperatingFrequency(router) })
            add("a 5 GHz channel" to { setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_5GHZ) })
            if (router != null && router < FIVE_GHZ) add("the router's 2.4 GHz channel $router MHz" to { setGroupOperatingFrequency(router) })
            add("any channel" to { setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_AUTO) })
        }
        var created = false
        for ((what, extra) in attempts) {
            if (manager.perform("create a group on $what") { createGroup(channel, config(name, passphrase, extra), it) }) {
                created = true
                break
            }
        }
        if (!created) return null
        repeat(25) {
            val group = groupInfo(manager, channel)
            if (group != null && group.isGroupOwner) {
                return WifiGroup(group.networkName, group.passphrase, group.frequency.toLong())
            }
            delay(200)
        }
        return null
    }

    /** Joins the receiver's group. Returns the receiver's address in it once the link is up, or null. */
    suspend fun join(group: WifiGroup): String? {
        val manager = manager ?: return null
        val channel = channel ?: return null
        val config = config(group.ssid, group.passphrase) { setGroupOperatingFrequency(group.frequency.toInt()) }
        if (!clear(manager, channel)) return null
        // Phones with one Wi-Fi channel may drop the router while joining; record it so a
        // test shows whether that happens (nothing is changed on the strength of it yet).
        AppLog.write("wifi-direct", "before joining: router on ${routerFrequency()?.let { "$it MHz" } ?: "no router"}")
        var joined = manager.perform("join ${group.ssid}") { connect(channel, config, it) }
        if (!joined) {
            // Android can still be settling after a group closes; one more try covers it.
            delay(1_000)
            joined = manager.perform("join ${group.ssid} again") { connect(channel, config, it) }
        }
        if (!joined) return null
        repeat(75) {
            val info = withTimeoutOrNull(ANSWER_MS) {
                suspendCancellableCoroutine<WifiP2pInfo?> { c -> manager.requestConnectionInfo(channel) { c.resume(it) } }
            }
            val owner = info?.groupOwnerAddress?.hostAddress
            if (info?.groupFormed == true && owner != null) {
                AppLog.write("wifi-direct", "after joining: router on ${routerFrequency()?.let { "$it MHz" } ?: "no router"}")
                return owner
            }
            delay(200)
        }
        return null
    }

    fun removeGroup() {
        channel?.let { manager?.removeGroup(it, null) }
    }

    /**
     * Android keeps a group running after the app that made it stops, for
     * example when Nearvolt updates itself. Removes one Nearvolt left behind;
     * groups of other apps, such as screen mirroring, are left alone.
     */
    suspend fun removeLeftover() {
        val manager = manager ?: return
        val channel = channel ?: return
        val group = groupInfo(manager, channel) ?: return
        if (group.networkName.startsWith(PREFIX)) {
            AppLog.write("wifi-direct", "removing a group left over from before")
            removeAndWait(manager, channel)
        }
    }

    /**
     * A phone is in one group at a time, so a leftover Nearvolt group goes before
     * a new one is made or joined. Returns false when another app's group is in
     * the way.
     */
    private suspend fun clear(manager: WifiP2pManager, channel: WifiP2pManager.Channel): Boolean {
        val group = groupInfo(manager, channel) ?: return true
        if (!group.networkName.startsWith(PREFIX)) {
            AppLog.write("wifi-direct", "another app's Wi-Fi Direct group is active, so Nearvolt can't use Wi-Fi Direct")
            return false
        }
        return removeAndWait(manager, channel)
    }

    /** Android reports a removal before it's done; new requests fail until the group is really gone. */
    private suspend fun removeAndWait(manager: WifiP2pManager, channel: WifiP2pManager.Channel): Boolean {
        manager.perform("remove the old group") { removeGroup(channel, it) }
        repeat(15) {
            if (groupInfo(manager, channel) == null) return true
            delay(200)
        }
        AppLog.write("wifi-direct", "the old group is still there after 3 s")
        return false
    }

    /** Closes the group unless a sender joined it, so an unused group doesn't run through a transfer. */
    suspend fun removeGroupIfUnused(senderAddress: String?) {
        val manager = manager ?: return
        val channel = channel ?: return
        val group = groupInfo(manager, channel) ?: return
        // The sender's own address says which way it came; the client list can still be
        // empty for a moment after a sender joins.
        val joined = if (senderAddress.isNullOrEmpty()) {
            group.clientList.isNotEmpty()
        } else {
            senderAddress.startsWith(GROUP_SUBNET)
        }
        if (!joined) {
            AppLog.write("wifi-direct", "closed the group: the sender came over the router or the cable")
            removeGroup()
        } else {
            AppLog.write("wifi-direct", "kept the group: the sender joined it")
        }
    }

    /** The group this phone is in, if any, for the log. */
    suspend fun report(): String? {
        val manager = manager ?: return null
        val channel = channel ?: return null
        val group = groupInfo(manager, channel) ?: return null
        return if (group.isGroupOwner) {
            "Wi-Fi Direct: own group, ${band(group.frequency)} (${group.frequency} MHz), ${group.clientList.size} joined"
        } else {
            "Wi-Fi Direct: joined a group, ${band(group.frequency)} (${group.frequency} MHz)"
        }
    }

    private suspend fun groupInfo(manager: WifiP2pManager, channel: WifiP2pManager.Channel): WifiP2pGroup? =
        withTimeoutOrNull(ANSWER_MS) {
            suspendCancellableCoroutine { c -> manager.requestGroupInfo(channel) { c.resume(it) } }
        }

    /**
     * The router's channel, if a group may use it. Channels shared with radar
     * (5250–5730 MHz) and 6 GHz are off limits to Wi-Fi Direct, so there the
     * group picks its own channel instead.
     */
    @Suppress("DEPRECATION") // connectionInfo is the only way to read the frequency without a network callback.
    private fun routerFrequency(): Int? =
        context.getSystemService(WifiManager::class.java)?.connectionInfo?.frequency
            ?.takeIf { it in 2_400..2_500 || it in 5_150..5_250 || it in 5_735..5_895 }

    private fun config(name: String, passphrase: String, extra: WifiP2pConfig.Builder.() -> Unit) =
        WifiP2pConfig.Builder().setNetworkName(name).setPassphrase(passphrase).apply(extra).build()

    /**
     * Android sometimes never answers, for example just after Wi-Fi turns on, so
     * every request has a time limit. "Busy" passes within a moment, so it's
     * asked again a few times.
     */
    private suspend fun WifiP2pManager.perform(
        what: String,
        action: WifiP2pManager.(WifiP2pManager.ActionListener) -> Unit,
    ): Boolean {
        repeat(BUSY_TRIES) { attempt ->
            val reason = withTimeoutOrNull(ANSWER_MS) {
                suspendCancellableCoroutine { continuation ->
                    action(object : WifiP2pManager.ActionListener {
                        override fun onSuccess() = continuation.resume(SUCCESS)
                        override fun onFailure(reason: Int) = continuation.resume(reason)
                    })
                }
            }
            when {
                reason == SUCCESS -> return true
                reason == WifiP2pManager.BUSY && attempt < BUSY_TRIES - 1 -> delay(BUSY_WAIT_MS)
                else -> {
                    AppLog.write("wifi-direct", "couldn't $what: ${reason?.let(::describe) ?: "no answer from Android"}")
                    return false
                }
            }
        }
        return false
    }

    private fun describe(reason: Int) = when (reason) {
        WifiP2pManager.BUSY -> "Android is busy"
        WifiP2pManager.P2P_UNSUPPORTED -> "this phone doesn't support Wi-Fi Direct"
        WifiP2pManager.ERROR -> "internal error"
        else -> "reason $reason"
    }

    companion object {
        fun band(frequency: Int) = when {
            frequency <= 0 -> "unknown band"
            frequency < 3_000 -> "2.4 GHz"
            frequency < 5_925 -> "5 GHz"
            else -> "6 GHz"
        }

        /** Every group Nearvolt makes is named this way, so its leftovers can be told apart. */
        private const val PREFIX = "DIRECT-NV-"

        /** Addresses Android gives a Wi-Fi Direct group's members. */
        private const val GROUP_SUBNET = "192.168.49."
        private const val FIVE_GHZ = 5_000
        private const val ANSWER_MS = 5_000L
        private const val BUSY_TRIES = 4
        private const val BUSY_WAIT_MS = 500L

        /** Not one of Android's failure reasons, which start at 0. */
        private const val SUCCESS = -1
    }

    private fun random(length: Int): String {
        val alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789"
        val random = SecureRandom()
        return String(CharArray(length) { alphabet[random.nextInt(alphabet.length)] })
    }
}
