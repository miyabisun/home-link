package dev.miyabisun.homelink

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.appwidget.AppWidgetManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/** The id this phone made up for itself, which home-link registers it by. */
internal object PhoneIdentity {
    fun id(context: Context): String {
        val prefs = context.getSharedPreferences("phone", Context.MODE_PRIVATE)
        return prefs.getString("id", null) ?: UUID.randomUUID().toString().also { prefs.edit().putString("id", it).commit() }
    }
}

internal fun Context.locationAllowed(): Boolean =
    checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

/** Where a location says the phone is, with how long ago it was measured. */
internal fun Location.fix() =
    Fix(latitude, longitude, accuracy.toDouble(), fixAgeMs(SystemClock.elapsedRealtimeNanos(), elapsedRealtimeNanos))

/**
 * Runs a routine widget's press as a foreground service, as [PressService] does a press, and for a routine that waits
 * for the phone to come near home, watches where the phone is until home-link takes it as near, the wait runs out or
 * the button is pressed again. The location is read only during that watch and let go the moment it ends.
 *
 * Started by the widget, the service may read the location with the "while using the app" permission alone.
 */
class SceneService : Service() {
    /** One watch for the phone to come near home; `wake` is released for each fix and for a cancel. */
    private class Watch(val wake: Semaphore = Semaphore(0)) {
        @Volatile var cancelled = false
    }

    companion object {
        private const val CHANNEL = "press"
        private const val NOTIFICATION = 2
        /** How often a fix is asked for while watching. */
        private const val FIX_EVERY_MS = 5_000L
        /** The presses running now by widget, each with its watch once it waits. */
        private val presses = ConcurrentHashMap<Int, AtomicReference<Watch?>>()
        private val flows = ConcurrentHashMap<Int, PressFlow>()
        private val running = AtomicInteger()

        /** Whether widget `id`'s routine waits for the phone to come near home now. */
        fun waiting(id: Int): Boolean = presses[id]?.get() != null

        /** What a routine widget's button sends: runs it, or takes back the wait it is in. */
        fun pending(context: Context, id: Int): PendingIntent =
            PendingIntent.getForegroundService(context, id, Intent(context, SceneService::class.java).setAction("scene:$id")
                .putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id), PendingIntent.FLAG_IMMUTABLE)
    }

    private val main = Handler(Looper.getMainLooper())

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val id = intent?.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
            ?: AppWidgetManager.INVALID_APPWIDGET_ID
        val name = SceneWidget.name(this, id)
        if (name == null) {
            if (running.get() == 0) stopSelf(startId)
            return START_NOT_STICKY
        }
        foreground("${name}を実行しています")
        val press = AtomicReference<Watch?>()
        val under = presses.putIfAbsent(id, press)
        if (under != null) {
            // Pressed again: a wait is taken back; a run on its way is left to end.
            under.get()?.let { watch ->
                watch.cancelled = true
                watch.wake.release()
            }
            return START_NOT_STICKY
        }
        running.incrementAndGet()
        Thread {
            try {
                press(id, name, press)
            } finally {
                presses.remove(id)
                main.post { if (running.decrementAndGet() == 0) stopSelf() }
            }
        }.start()
        return START_NOT_STICKY
    }

    /** Shows `title` as the service's notification, in the foreground as a location service when that is allowed. */
    private fun foreground(title: String) {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.press_channel), NotificationManager.IMPORTANCE_LOW))
        val notification = Notification.Builder(this, CHANNEL).setSmallIcon(R.drawable.ic_power).setContentTitle(title).build()
        val type = if (locationAllowed()) ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION else ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE
        try {
            startForeground(NOTIFICATION, notification, type)
        } catch (e: RuntimeException) {
            // Refused: sent from the background, where Data Saver may block it and no location is read.
            Log.w(LOG, "scene: not in the foreground as type $type: $e")
        }
    }

    private fun press(id: Int, name: String, press: AtomicReference<Watch?>) {
        flows.getOrPut(id) { PressFlow() }.run(this, sending = { SceneWidget.show(this, id, SENDING, sending = true) }, send = {
            val api = homeLinkApi(this)
            val phone = PhoneIdentity.id(this)
            var result = api.runScene(name, 0, phone, null)
            var end: WatchEnd? = null
            val run = (result as? ApiResult.Ok)?.value
            if (run is SceneRun.Waiting && run.watchable) {
                val watched = watch(id, name, run, api, phone, press)
                result = watched.first
                end = watched.second
            }
            Log.i(LOG, "scene $name: ${end ?: (result as? ApiResult.Ok)?.value?.javaClass?.simpleName ?: result}")
            SceneWidget.show(this, id, end?.label ?: scenePress(result),
                message = end?.message?.format(name) ?: sceneMessage(name, result, homeLinkHost(this)))
            true
        }, reread = {}, restore = { SceneWidget.show(this, id) })
    }

    /**
     * Watches where the phone is and has home-link go on with `name` from the step it waits at for each fix, until it
     * does, says the wait cannot end, the wait's minutes run out, or the button is pressed again.
     */
    private fun watch(id: Int, name: String, waiting: SceneRun.Waiting, api: HomeLinkApi, phone: String,
                      press: AtomicReference<Watch?>): Pair<ApiResult<SceneRun>, WatchEnd?> {
        val first: ApiResult<SceneRun> = ApiResult.Ok(waiting)
        if (!locationAllowed()) return first to WatchEnd.NO_PERMISSION
        val manager = getSystemService(LocationManager::class.java)
        if (!manager.isLocationEnabled) return first to WatchEnd.LOCATION_OFF
        val providers = listOf(LocationManager.GPS_PROVIDER, LocationManager.FUSED_PROVIDER, LocationManager.NETWORK_PROVIDER)
            .filter { manager.hasProvider(it) && manager.isProviderEnabled(it) }
        if (providers.isEmpty()) return first to WatchEnd.LOCATION_OFF

        val watch = Watch()
        val latest = AtomicReference<Location?>()
        val listener = LocationListener { location ->
            latest.set(location)
            watch.wake.release()
        }
        val deadline = SystemClock.elapsedRealtime() + waiting.waitMinutes * 60_000L
        press.set(watch)
        SceneWidget.show(this, id)
        foreground("${name}：自宅の近くに着くのを待っています")
        // Should the process die while it waits, the alarm takes the button back.
        Widgets.restoreAt(this, deadline)
        try {
            for (provider in providers) manager.requestLocationUpdates(provider, FIX_EVERY_MS, 0f, { it.run() }, listener)
        } catch (e: SecurityException) {
            Log.w(LOG, "scene $name: the location is not to be read: $e")
            press.set(null)
            return first to WatchEnd.NO_PERMISSION
        }
        try {
            var next = waiting.next
            var sendAfter = 0L
            while (true) {
                val left = deadline - SystemClock.elapsedRealtime()
                if (left <= 0 || !watch.wake.tryAcquire(left, TimeUnit.MILLISECONDS)) return first to WatchEnd.EXPIRED
                watch.wake.drainPermits()
                if (watch.cancelled) return first to WatchEnd.CANCELLED
                if (SystemClock.elapsedRealtime() < sendAfter) continue
                val location = latest.getAndSet(null) ?: continue
                when (val result = api.runScene(name, next, phone, location.fix())) {
                    is ApiResult.Ok -> when (val run = result.value) {
                        is SceneRun.Done -> return result to null
                        is SceneRun.Waiting -> {
                            if (!run.watchable) return result to null
                            next = run.next
                            sendAfter = SystemClock.elapsedRealtime() + resendAfterMs(run.distance)
                        }
                    }
                    // On the road a request may not get through; the next fix tries again.
                    is ApiResult.Failed -> if (result.error == ApiError.SCENE_NOT_FOUND) return result to null
                }
            }
        } finally {
            manager.removeUpdates(listener)
            press.set(null)
        }
    }

    // A short service may run 3 minutes; a routine without a wait takes less.
    override fun onTimeout(startId: Int, fgsType: Int) = stopSelf()
}
