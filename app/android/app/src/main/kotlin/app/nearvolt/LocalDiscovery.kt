package app.nearvolt

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.CompletableDeferred

/**
 * Finds receivers on the same Wi-Fi network with mDNS. A receiver advertises its
 * public ticket in TXT records, so the sender can connect without another step.
 */
class LocalDiscovery(context: Context, private val emit: (PlatformEvent) -> Unit) {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private var registration: NsdManager.RegistrationListener? = null
    private var browsing: NsdManager.DiscoveryListener? = null
    private val toResolve = ArrayDeque<NsdServiceInfo>()
    private var resolving = false
    private val retry = Handler(Looper.getMainLooper())

    /** Set by [stopBrowsing]; a retry still waiting then does nothing. */
    @Volatile
    private var stopped = false

    /**
     * Lists this phone for others on the network. The result says whether Android
     * accepted it, which can fail quietly (too many listeners, no multicast).
     */
    fun advertise(name: String, kind: String, ticket: String, port: Int): CompletableDeferred<Boolean> {
        stopAdvertising()
        val listed = CompletableDeferred<Boolean>()
        val info = NsdServiceInfo().apply {
            serviceName = name
            serviceType = SERVICE_TYPE
            setPort(port)
            setAttribute("k", kind)
            // TXT values are limited to 255 bytes, so long tickets are split.
            ticket.chunked(TXT_CHUNK).forEachIndexed { i, part -> setAttribute("t$i", part) }
        }
        registration = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(info: NsdServiceInfo) {
                listed.complete(true)
            }

            override fun onRegistrationFailed(info: NsdServiceInfo, error: Int) {
                AppLog.write("nearby", "couldn't list this phone on the network (error $error)")
                listed.complete(false)
            }

            override fun onServiceUnregistered(info: NsdServiceInfo) {}
            override fun onUnregistrationFailed(info: NsdServiceInfo, error: Int) {}
        }.also { nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, it) }
        return listed
    }

    fun stopAdvertising() {
        registration?.let { runCatching { nsd.unregisterService(it) } }
        registration = null
    }

    fun browse(attempt: Int = 1) {
        if (attempt == 1) stopped = false
        if (browsing != null) return
        browsing = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}

            override fun onStartDiscoveryFailed(serviceType: String, error: Int) {
                AppLog.write("nearby", "searching the network failed (error $error), attempt $attempt")
                browsing = null
                // Android sometimes refuses at first, for example after too many searches
                // in a row; a couple of retries usually get through. Then the app is told.
                if (attempt < SEARCH_ATTEMPTS) {
                    retry.postDelayed({ if (!stopped) browse(attempt + 1) }, RETRY_MS)
                } else {
                    emit(PlatformEvent(PlatformEventKind.SEARCH_FAILED))
                }
            }

            override fun onStopDiscoveryFailed(serviceType: String, error: Int) {}
            override fun onServiceFound(info: NsdServiceInfo) {
                synchronized(this@LocalDiscovery) { toResolve += info }
                resolveNext()
            }

            override fun onServiceLost(info: NsdServiceInfo) {
                emit(PlatformEvent(PlatformEventKind.RECEIVER_LOST, id = id(info)))
            }
        }.also { nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, it) }
    }

    fun stopBrowsing() {
        stopped = true
        retry.removeCallbacksAndMessages(null)
        browsing?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        browsing = null
        synchronized(this) { toResolve.clear() }
    }

    /** NsdManager resolves one service at a time; callbacks arrive on other threads. */
    @Suppress("DEPRECATION") // resolveService still works and covers Android 11 to 13 too.
    @Synchronized
    private fun resolveNext() {
        if (resolving) return
        val next = toResolve.removeFirstOrNull() ?: return
        resolving = true
        nsd.resolveService(next, object : NsdManager.ResolveListener {
            override fun onResolveFailed(info: NsdServiceInfo, error: Int) = done()

            override fun onServiceResolved(info: NsdServiceInfo) {
                val parts = generateSequence(0) { it + 1 }
                    .map { info.attributes["t$it"] }
                    .takeWhile { it != null }
                    .joinToString("") { String(it!!) }
                if (parts.isNotEmpty()) {
                    emit(
                        PlatformEvent(
                            PlatformEventKind.RECEIVER_FOUND,
                            id = id(info),
                            name = info.serviceName,
                            deviceKind = info.attributes["k"]?.let(::String) ?: "phone",
                            link = "Same Wi-Fi",
                            ticket = parts,
                        ),
                    )
                }
                done()
            }

            private fun done() {
                synchronized(this@LocalDiscovery) { resolving = false }
                resolveNext()
            }
        })
    }

    private fun id(info: NsdServiceInfo) = "lan:${info.serviceName}"

    private companion object {
        const val SEARCH_ATTEMPTS = 3
        const val RETRY_MS = 2_000L
        const val SERVICE_TYPE = "_nearvolt._tcp"
        const val TXT_CHUNK = 200
    }
}
