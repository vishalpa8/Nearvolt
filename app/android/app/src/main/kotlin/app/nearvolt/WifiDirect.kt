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
     * Creates a group this device owns. When connected to Wi-Fi it uses the
     * router's channel, so the radio never has to hop between two channels.
     */
    suspend fun createGroup(): WifiGroup? {
        val manager = manager ?: return null
        val channel = channel ?: return null
        removeGroup()
        val name = "DIRECT-NV-" + random(4)
        val passphrase = random(12)
        val created = routerFrequency()?.let { frequency ->
            manager.perform { createGroup(channel, config(name, passphrase) { setGroupOperatingFrequency(frequency) }, it) }
        } ?: false
        if (!created) {
            val auto = config(name, passphrase) { setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_AUTO) }
            if (!manager.perform { createGroup(channel, auto, it) }) return null
        }
        repeat(25) {
            val group = suspendCancellableCoroutine<WifiP2pGroup?> { c -> manager.requestGroupInfo(channel) { c.resume(it) } }
            if (group != null && group.isGroupOwner) {
                return WifiGroup(group.networkName, group.passphrase, group.frequency.toLong())
            }
            delay(200)
        }
        return null
    }

    /** Joins the receiver's group. Returns once the link is up. */
    suspend fun join(group: WifiGroup): Boolean {
        val manager = manager ?: return false
        val channel = channel ?: return false
        val config = config(group.ssid, group.passphrase) { setGroupOperatingFrequency(group.frequency.toInt()) }
        if (!manager.perform { connect(channel, config, it) }) return false
        repeat(75) {
            val info = suspendCancellableCoroutine<WifiP2pInfo?> { c -> manager.requestConnectionInfo(channel) { c.resume(it) } }
            if (info?.groupFormed == true) return true
            delay(200)
        }
        return false
    }

    fun removeGroup() {
        channel?.let { manager?.removeGroup(it, null) }
    }

    /** Closes the group unless a sender joined it, so an unused group doesn't run through a transfer. */
    suspend fun removeGroupIfUnused() {
        val manager = manager ?: return
        val channel = channel ?: return
        val group = suspendCancellableCoroutine<WifiP2pGroup?> { c -> manager.requestGroupInfo(channel) { c.resume(it) } }
        if (group != null && group.clientList.isEmpty()) removeGroup()
    }

    @Suppress("DEPRECATION") // connectionInfo is the only way to read the frequency without a network callback.
    private fun routerFrequency(): Int? =
        context.getSystemService(WifiManager::class.java)?.connectionInfo?.frequency?.takeIf { it > 0 }

    private fun config(name: String, passphrase: String, extra: WifiP2pConfig.Builder.() -> Unit) =
        WifiP2pConfig.Builder().setNetworkName(name).setPassphrase(passphrase).apply(extra).build()

    private suspend fun WifiP2pManager.perform(action: WifiP2pManager.(WifiP2pManager.ActionListener) -> Unit): Boolean =
        suspendCancellableCoroutine { continuation ->
            action(object : WifiP2pManager.ActionListener {
                override fun onSuccess() = continuation.resume(true)
                override fun onFailure(reason: Int) = continuation.resume(false)
            })
        }

    private fun random(length: Int): String {
        val alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789"
        val random = SecureRandom()
        return String(CharArray(length) { alphabet[random.nextInt(alphabet.length)] })
    }
}
