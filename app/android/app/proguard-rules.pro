# ML Kit (Google's code scanner) creates its component registrars by reflection.
# The rule shipped with firebase-components 16.1.0 keeps the classes but not
# their constructors, which R8 full mode then removes, and the scanner crashes.
-keep class * implements com.google.firebase.components.ComponentRegistrar { <init>(); }
