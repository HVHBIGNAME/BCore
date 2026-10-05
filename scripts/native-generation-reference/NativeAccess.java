import java.lang.reflect.*;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

/** JDK-21-compilable access to the pinned JDK-25 engine, with no game substitutes. */
final class NativeAccess {
    private static final Map<String, Method> METHODS = new ConcurrentHashMap<>();
    private static final Map<String, Field> FIELDS = new ConcurrentHashMap<>();

    static Class<?> type(String name) throws Exception {
        return Class.forName(name.startsWith("net.") ? name : "net.minecraft." + name);
    }

    static Class<?> boxed(Class<?> type) {
        if (!type.isPrimitive()) return type;
        if (type == int.class) return Integer.class;
        if (type == long.class) return Long.class;
        if (type == boolean.class) return Boolean.class;
        if (type == float.class) return Float.class;
        if (type == double.class) return Double.class;
        if (type == short.class) return Short.class;
        if (type == byte.class) return Byte.class;
        if (type == char.class) return Character.class;
        return Void.class;
    }

    static boolean matches(Class<?>[] types, Object[] args) {
        if (types.length != args.length) return false;
        for (int i = 0; i < types.length; i++) {
            if (args[i] == null ? types[i].isPrimitive() : !boxed(types[i]).isInstance(args[i])) return false;
        }
        return true;
    }

    static Object call(Object target, String name, Object... args) throws Exception {
        Class<?> owner = target instanceof Class<?> c ? c : target.getClass();
        String key = owner.getName() + "." + name + Arrays.toString(Arrays.stream(args)
            .map(a -> a == null ? "null" : a.getClass().getName()).toArray());
        Method method = METHODS.get(key);
        if (method == null) {
            search: for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
                for (Method m : c.getDeclaredMethods()) {
                    if (m.getName().equals(name) && matches(m.getParameterTypes(), args)) {
                        method = m;
                        break search;
                    }
                }
            }
            if (method == null) for (Method m : owner.getMethods()) {
                if (m.getName().equals(name) && matches(m.getParameterTypes(), args)) {
                    method = m;
                    break;
                }
            }
            if (method == null) throw new NoSuchMethodException(key);
            method.setAccessible(true);
            METHODS.put(key, method);
        }
        try {
            return method.invoke(target instanceof Class<?> ? null : target, args);
        } catch (InvocationTargetException e) {
            if (e.getCause() instanceof Exception cause) throw cause;
            if (e.getCause() instanceof Error cause) throw cause;
            throw e;
        }
    }

    static Object field(Object target, String name) throws Exception {
        Class<?> owner = target instanceof Class<?> c ? c : target.getClass();
        String key = owner.getName() + "." + name;
        Field field = FIELDS.get(key);
        if (field == null) {
            for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
                try {
                    field = c.getDeclaredField(name);
                    break;
                } catch (NoSuchFieldException ignored) { }
            }
            if (field == null) throw new NoSuchFieldException(key);
            field.setAccessible(true);
            FIELDS.put(key, field);
        }
        return field.get(target instanceof Class<?> ? null : target);
    }

    static Object constant(String type, String name) throws Exception {
        return field(type(type), name);
    }

    static Object make(String name, Object... args) throws Exception {
        for (Constructor<?> c : type(name).getDeclaredConstructors()) {
            if (matches(c.getParameterTypes(), args)) {
                c.setAccessible(true);
                return c.newInstance(args);
            }
        }
        throw new NoSuchMethodException(name + " constructor");
    }

    static Map<String, Object> map(Object... pairs) {
        Map<String, Object> out = new LinkedHashMap<>();
        for (int i = 0; i < pairs.length; i += 2) out.put((String) pairs[i], pairs[i + 1]);
        return out;
    }
}
