import { mergeNamespaces, type MessageTree } from "../namespaces";

// One module per locale (a glob pattern must be a literal), so each locale is one lazy chunk.
export default mergeNamespaces(import.meta.glob<MessageTree>("../../locales/cs/*.json", { eager: true, import: "default" }));
