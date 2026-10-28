// LGPL-3.0-or-later: methods inserted at runtime are from AE2 19.2.18,
// GenericStack.fromItemStack/unwrapItemStack, WrappedGenericStack.unwrapWhat/
// unwrapAmount, and AECraftingPattern.getCraftingInputs. Stubs model only the
// item count, component and key operations those methods require.
import java.util.Arrays;
import java.util.List;
import java.util.Objects;

interface AEKey {}
record AEItemKey(String id, String components) implements AEKey {
    static AEItemKey of(ItemStack stack) { return stack.isEmpty() ? null : stack.key; }
}
record AEFluidKey(String id) implements AEKey {}
class Item {}
class AEComponents { static final Object WRAPPED_STACK = new Object(); }
class ItemStack {
    final Item item;
    final int count;
    final AEItemKey key;
    final GenericStack wrapped;
    ItemStack(Item item, int count, AEItemKey key, GenericStack wrapped) {
        this.item = item; this.count = count; this.key = key; this.wrapped = wrapped;
    }
    boolean isEmpty() { return count <= 0; }
    Item getItem() { return item; }
    int getCount() { return count; }
    GenericStack get(Object component) { return component == AEComponents.WRAPPED_STACK ? wrapped : null; }
}
class Preconditions {
    static void checkArgument(boolean condition, String message) {
        if (!condition) throw new IllegalArgumentException(message);
    }
}
class WrappedGenericStack extends Item {
    // ACTUAL_UNWRAP_WHAT
    // ACTUAL_UNWRAP_AMOUNT
}
record GenericStack(AEKey what, long amount) {
    // ACTUAL_FROM_ITEM
    // ACTUAL_UNWRAP_ITEM
}
class AECraftingPattern {
    // ACTUAL_CRAFTING_INPUTS
}
public class SourceWrappedOracle {
    static void eq(Object actual, Object expected, String name) {
        if (!Objects.equals(actual, expected)) throw new AssertionError(name + ": " + actual + " != " + expected);
        System.out.println(name + "=1");
    }
    public static void main(String[] args) {
        var oak = new AEItemKey("minecraft:oak_log", "variant=wrapped");
        var wrapperKey = new AEItemKey("ae2:wrapped_generic_stack", "no-component");
        var ordinary = new Item();
        var wrapper = new WrappedGenericStack();
        var inner = new GenericStack(oak, 1);
        var fluid = new GenericStack(new AEFluidKey("minecraft:water"), 1000);
        var entries = List.of(
            new ItemStack(ordinary, 1, oak, null),
            new ItemStack(wrapper, 1, wrapperKey, inner),
            new ItemStack(wrapper, 4, wrapperKey, inner),
            new ItemStack(wrapper, 1, wrapperKey, new GenericStack(oak, 2)),
            new ItemStack(wrapper, 0, wrapperKey, inner),
            new ItemStack(wrapper, 1, wrapperKey, null),
            new ItemStack(wrapper, 1, wrapperKey, fluid)
        );
        var actual = AECraftingPattern.getCraftingInputs(entries);
        eq(actual.get(0), new GenericStack(oak, 1), "ordinary");
        eq(actual.get(1), inner, "wrapped_key_and_components");
        eq(actual.get(2), inner, "wrapped_outer_count_ignored");
        eq(actual.get(3), new GenericStack(oak, 2), "wrapped_inner_amount_retained");
        eq(actual.get(4), null, "empty_wrapper");
        eq(actual.get(5), new GenericStack(wrapperKey, 1), "degenerate_wrapper");
        eq(actual.get(6), fluid, "wrapped_fluid_key");
        try { AECraftingPattern.getCraftingInputs(Arrays.asList(new ItemStack[10])); throw new AssertionError("accepted 10 slots"); }
        catch (IllegalArgumentException expected) { System.out.println("slot_limit=1"); }
    }
}
