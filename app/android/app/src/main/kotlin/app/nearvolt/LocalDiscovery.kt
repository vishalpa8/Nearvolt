package app.nearvolt

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo

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

    fun advertise(name: String, kind: String, ticket: String, port: Int) {
        stopAdvertising()
        val info = NsdServiceInfo().apply {
            serviceName = name
            serviceType = SERVICE_TYPE
            setPort(port)
            setAttribute("k", kind)
            // TXT values are limited to 255 bytes, so long tickets are split.
            ticket.chunked(TXT_CHUNK).forEachIndexed { i, part -> setAttribute("t$i", part) }
        }
        registration = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(info: NsdServiceInfo) {}
            override fun onRegistrationFailed(info: NsdServiceInfo, error: Int) {}
            override fun onServiceUnregistered(info: NsdServiceInfo) {}
            override fun onUnregistrationFailed(info: NsdServiceInfo, error: Int) {}
        }.also { nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, it) }
    }

    fun stopAdvertising() {
        registration?.let { runCatching { nsd.unregisterService(it) } }
        registration = null
    }

    fun browse() {
        if (browsing != null) return
        browsing = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}
            override fun onStartDiscoveryFailed(serviceType: String, error: Int) {}
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
        const val SERVICE_TYPE = "_nearvolt._tcp"
        const val TXT_CHUNK = 200
    }
}
