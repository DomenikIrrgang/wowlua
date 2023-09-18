import { DependencyType } from "./dependency-type";

export interface Dependency {
    name: string,
    source: string,
    type: DependencyType,
    version?: string,
    path?: string
}
